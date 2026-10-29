// Git 쓰기 — commit · pull · fetch · push · revert(restore) · resolve · revert commit (2026.10.29~).
//
// 「설정 › 버전 관리 › Allow write」를 켠 서버에서 사용자가 단추를 누를 때만 부른다. 그 검사는 부르는 쪽(main.rs)이 한다.
// 읽기(status · log · blame · 충돌 세 쪽)는 git.rs 에 있다.
//
//   GIT_OPTIONAL_LOCKS 는 걸지 않는다 — 쓰기 명령은 색인을 잠그고 고쳐야 한다
//   GIT_TERMINAL_PROMPT=0  pull 이 계정을 묻느라 멈추지 않게 한다. 원격 계정은 git 의 자격 증명 관리자에 맡긴다
//   Pull 은 「설정 › 버전 관리 › Pull 명령」(기본 git pull --ff-only)이다. 갈라졌으면 git 이 멈추고, 실행기는 합치거나 리베이스하지 않는다.
//   Push 는 지금 브랜치를 그 원격 추적 브랜치(@{u})에만 올린다. 원격 · 브랜치를 추측해 새로 만들지 않고 --force 는 어디에도 없다.

use std::fs;
use std::path::Path;
use std::process::Command;
use std::time::Duration;

use crate::svn::decode;
use crate::svncli::{run, temp_message};

const LOCAL_LIMIT: Duration = Duration::from_secs(120);
const PULL_LIMIT: Duration = Duration::from_secs(300);

fn git_w(root: &Path, args: &[&str], limit: Duration) -> Result<String, String> {
    let mut c = Command::new("git");
    c.arg("-C").arg(root).args(["-c", "core.quotePath=false", "-c", "color.ui=never"]).args(args).env("GIT_TERMINAL_PROMPT", "0");
    let out = run(c, None, limit).map_err(|e| format!("git 을 실행하지 못했습니다. git 이 PATH 에 있어야 합니다 ({e})."))?;
    if !out.ok {
        return Err(if out.stderr.is_empty() { format!("git {} 이 실패했습니다.", args[0]) } else { out.stderr });
    }
    Ok(decode(&out.stdout))
}

fn with_paths<'a>(head: &[&'a str], paths: &'a [String]) -> Vec<&'a str> {
    let mut v = head.to_vec();
    v.push("--");
    v.extend(paths.iter().map(String::as_str));
    v
}

/// 고른 경로만 Commit — 이름을 바꾼 파일은 옛 경로도 paths 에 함께 온다. 이미 스테이지된 다른 파일은 그대로 남는다.
pub fn commit(root: &Path, msg: &str, paths: &[String]) -> Result<String, String> {
    git_w(root, &with_paths(&["add", "-A"], paths), LOCAL_LIMIT)?;
    let file = temp_message(msg)?;
    let f = file.to_string_lossy().into_owned();
    let r = git_w(root, &with_paths(&["commit", "-F", f.as_str()], paths), LOCAL_LIMIT);
    let _ = fs::remove_file(&file);
    r?;
    Ok(git_w(root, &["rev-parse", "--short", "HEAD"], LOCAL_LIMIT)?.trim().to_string())
}

/// 설정의 Pull 명령 — 「git 」 으로 시작하는 한 줄만 받는다 (셸을 거치지 않는다)
pub fn pull_args(cmd: &str) -> Result<Vec<String>, String> {
    let mut it = cmd.split_whitespace();
    if it.next() != Some("git") {
        return Err(format!("Pull 명령은 git 으로 시작해야 합니다: {cmd}"));
    }
    let rest: Vec<String> = it.map(String::from).collect();
    if rest.is_empty() {
        return Err("Pull 명령이 비었습니다.".into());
    }
    Ok(rest)
}

pub fn pull(root: &Path, cmd: &str) -> Result<String, String> {
    let args = pull_args(cmd)?;
    let refs: Vec<&str> = args.iter().map(String::as_str).collect();
    let out = git_w(root, &refs, PULL_LIMIT)?;
    Ok(out.lines().filter(|l| !l.trim().is_empty()).last().unwrap_or("").trim().to_string())
}

/// 원격 추적 브랜치만 새로 고친다 — 작업 파일은 바뀌지 않고 Incoming 이 최신이 된다
pub fn fetch(root: &Path) -> Result<(), String> {
    git_w(root, &["fetch", "--prune"], PULL_LIMIT)?;
    Ok(())
}

/// 지금 브랜치가 올라갈 곳 (원격 이름 · 원격 브랜치 refs/heads/…). 없으면 Push 하지 않는다.
pub fn push_target(root: &Path) -> Result<(String, String, String), String> {
    let branch = git_w(root, &["symbolic-ref", "--quiet", "--short", "HEAD"], LOCAL_LIMIT)
        .map_err(|_| "분리된 HEAD 입니다. 브랜치로 옮긴 뒤 Push 하세요.".to_string())?
        .trim()
        .to_string();
    let get = |k: &str| git_w(root, &["config", "--get", &format!("branch.{branch}.{k}")], LOCAL_LIMIT).map(|s| s.trim().to_string()).ok().filter(|s| !s.is_empty());
    match (get("remote"), get("merge")) {
        (Some(remote), Some(merge)) if remote != "." => Ok((branch, remote, merge)),
        _ => Err(format!("{branch} 에 원격 추적 브랜치가 없습니다. 처음 올리는 브랜치는 git push -u 로 직접 올리세요.")),
    }
}

/// 지금 브랜치를 원격 추적 브랜치에 올린다. 원격이 앞서 있으면 git 이 거절하고 그 글을 그대로 돌려준다.
pub fn push(root: &Path) -> Result<String, String> {
    let (branch, remote, merge) = push_target(root)?;
    let spec = format!("HEAD:{merge}");
    git_w(root, &["push", remote.as_str(), spec.as_str()], PULL_LIMIT)?;
    Ok(format!("{branch} → {remote}/{}", merge.trim_start_matches("refs/heads/")))
}

/// 되돌릴 것 하나 — code 는 status 의 표시 (A 새로 더함 · 그 밖은 HEAD 로 되돌림), orig 는 이름을 바꾼 파일의 옛 경로
pub struct RevertItem {
    pub path: String,
    pub code: String,
    pub orig: Option<String>,
}

/// 새로 더한 파일은 색인에서만 빼고 파일은 남긴다 (SVN 의 Revert 와 같다). 나머지는 HEAD 내용으로 되돌린다.
pub fn revert(root: &Path, items: &[RevertItem]) -> Result<(), String> {
    let mut unstage: Vec<String> = Vec::new();
    let mut restore: Vec<String> = Vec::new();
    for it in items {
        if it.code == "?" {
            continue;
        }
        if it.code == "A" || it.orig.is_some() {
            unstage.push(it.path.clone());
            restore.extend(it.orig.clone());
        } else {
            restore.push(it.path.clone());
        }
    }
    if !unstage.is_empty() {
        git_w(root, &with_paths(&["rm", "--cached", "--quiet", "-r"], &unstage), LOCAL_LIMIT)?;
    }
    if !restore.is_empty() {
        git_w(root, &with_paths(&["restore", "--source=HEAD", "--staged", "--worktree"], &restore), LOCAL_LIMIT)?;
    }
    Ok(())
}

/// accept : mine(--ours) · theirs(--theirs) · working(고친 대로). 어느 쪽이든 끝에 git add 로 해결 표시를 한다.
pub fn resolve(root: &Path, rel: &str, accept: &str) -> Result<(), String> {
    let p = [rel.to_string()];
    match accept {
        "mine" => {
            git_w(root, &with_paths(&["checkout", "--ours"], &p), LOCAL_LIMIT)?;
        }
        "theirs" => {
            git_w(root, &with_paths(&["checkout", "--theirs"], &p), LOCAL_LIMIT)?;
        }
        "working" => {}
        x => return Err(format!("풀이 방법이 올바르지 않습니다: {x}")),
    }
    git_w(root, &with_paths(&["add"], &p), LOCAL_LIMIT)?;
    Ok(())
}

fn is_commit_id(id: &str) -> bool {
    id.len() >= 7 && id.bytes().all(|b| b.is_ascii_hexdigit())
}

/// 그 커밋이 바꾼 파일 (저장소 맨 위 기준)
pub fn changed_paths(root: &Path, id: &str) -> Result<Vec<String>, String> {
    if !is_commit_id(id) {
        return Err(format!("커밋 표기가 올바르지 않습니다: {id}"));
    }
    let out = git_w(root, &["diff-tree", "--no-commit-id", "--name-only", "-r", "-z", "--root", id], LOCAL_LIMIT)?;
    Ok(out.split('\0').filter(|s| !s.is_empty()).map(String::from).collect())
}

/// 커밋 하나를 거꾸로 적용만 한다 (git revert --no-commit). Commit 은 따로 한다.
pub fn revert_commit(root: &Path, id: &str) -> Result<(), String> {
    if !is_commit_id(id) {
        return Err(format!("커밋 표기가 올바르지 않습니다: {id}"));
    }
    let parents = git_w(root, &["rev-list", "--parents", "-n", "1", id], LOCAL_LIMIT)?;
    if parents.split_whitespace().count() > 2 {
        return Err("합치기 커밋(merge commit)은 되돌리지 않습니다. 어느 부모 기준인지 정해 git revert -m 으로 직접 하세요.".into());
    }
    // 스테이지된 변경이 있으면 되돌린 변경과 섞여 어느 것이 무엇인지 가리지 못한다
    if git_w(root, &["diff", "--cached", "--quiet"], LOCAL_LIMIT).is_err() {
        return Err("스테이지된 변경이 있습니다. 먼저 Commit 하거나 스테이지에서 빼 주세요.".into());
    }
    git_w(root, &["revert", "--no-commit", id], LOCAL_LIMIT)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pull_명령을_가린다() {
        assert_eq!(pull_args("git pull --ff-only").unwrap(), ["pull", "--ff-only"]);
        assert!(pull_args("svn update").is_err());
        assert!(pull_args("git").is_err());
        assert!(pull_args("").is_err());
    }

    #[test]
    fn 경로_앞에_구분자를_둔다() {
        let p = vec!["-a.txt".to_string(), "src/주문.java".to_string()];
        assert_eq!(with_paths(&["add", "-A"], &p), ["add", "-A", "--", "-a.txt", "src/주문.java"]);
        assert!(is_commit_id("abc1234") && !is_commit_id("HEAD~1") && !is_commit_id("-x"));
    }

    // 시험 준비용 — 임시 폴더 안에서만 부른다
    fn sh(dir: &Path, args: &[&str]) {
        let ok = Command::new("git").arg("-C").arg(dir).args(args).output().unwrap().status.success();
        assert!(ok, "git {args:?}");
    }

    #[test]
    fn 임시_저장소에서_commit_fetch_pull_push_를_한다() {
        let t = std::env::temp_dir().join(format!("tomcatrun-gitwrite-{}", std::process::id()));
        let _ = fs::remove_dir_all(&t);
        fs::create_dir_all(&t).unwrap();
        let (bare, w1, w2) = (t.join("remote.git"), t.join("w1"), t.join("w2"));
        sh(&t, &["init", "--bare", "-b", "main", bare.to_str().unwrap()]);
        sh(&t, &["clone", "-q", bare.to_str().unwrap(), w1.to_str().unwrap()]);
        sh(&w1, &["config", "user.name", "t"]);
        sh(&w1, &["config", "user.email", "t@t"]);
        // 빈 저장소를 받으면 첫 브랜치 이름이 이 PC 의 init.defaultBranch 를 따르므로 main 으로 못 박는다
        sh(&w1, &["symbolic-ref", "HEAD", "refs/heads/main"]);
        fs::write(w1.join("seed.txt"), "seed").unwrap();
        sh(&w1, &["add", "seed.txt"]);
        sh(&w1, &["commit", "-q", "-m", "seed"]);
        sh(&w1, &["push", "-q", "-u", "origin", "main"]);
        sh(&t, &["clone", "-q", bare.to_str().unwrap(), w2.to_str().unwrap()]);

        // 고른 파일만 Commit · 메시지의 따옴표 · 줄바꿈 · 한글이 그대로 · 따로 스테이지한 파일은 남는다
        fs::write(w1.join("a.txt"), "a").unwrap();
        fs::write(w1.join("c.txt"), "c").unwrap();
        sh(&w1, &["add", "c.txt"]);
        let msg = "FDR-1 \"주문\" 목록을 고친다\n\n본문 & %PATH%";
        commit(&w1, msg, &["a.txt".to_string()]).unwrap();
        assert_eq!(git_w(&w1, &["log", "-1", "--format=%B"], LOCAL_LIMIT).unwrap().trim_end(), msg);
        assert_eq!(git_w(&w1, &["diff", "--cached", "--name-only"], LOCAL_LIMIT).unwrap().trim(), "c.txt");

        // 올라갈 커밋이 보이고, Push 하면 비고, 받는 쪽은 Fetch → Pull 로 따라온다
        assert_eq!(crate::git::outgoing(&w1).unwrap().entries.len(), 1);
        assert_eq!(push(&w1).unwrap(), "main → origin/main");
        assert!(crate::git::outgoing(&w1).unwrap().entries.is_empty());
        fetch(&w2).unwrap();
        assert_eq!(crate::git::incoming(&w2).unwrap().entries.len(), 1);
        pull(&w2, "git pull --ff-only").unwrap();
        assert!(w2.join("a.txt").exists());

        // 원격 추적 브랜치가 없는 새 브랜치는 Push 하지 않는다
        sh(&w1, &["checkout", "-q", "-b", "new"]);
        assert!(push(&w1).unwrap_err().contains("원격 추적 브랜치가 없습니다"));
        let _ = fs::remove_dir_all(&t);
    }
}
