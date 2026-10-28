// 읽기 전용 Git — 저장소를 바꾸지 않는 git 명령만 부른다 (status · cat-file · log · rev-parse).
//
// fetch · pull · commit · checkout 처럼 저장소나 작업 폴더를 바꾸는 명령은 부르지 않는다. 화면은 명령을 복사해 줄 뿐이다.
//   GIT_OPTIONAL_LOCKS=0   git status 가 색인(.git/index)을 새로 고쳐 쓰지 않게 한다
//   GIT_TERMINAL_PROMPT=0  계정을 묻느라 멈추지 않게 한다 (원격에 묻는 명령은 쓰지 않지만 혹시 모를 때)
// 「업데이트 확인」은 원격에 묻지 않고, 마지막으로 fetch 해 둔 원격 추적 브랜치(@{u})와 견준다.
// 파일 내용은 cat-file --filters 로 꺼낸다. 줄끝 변환(core.autocrlf)을 거친 내용이라 작업 파일과 줄끝이 어긋나지 않는다.

use std::fs;
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Instant, UNIX_EPOCH};

use serde::Serialize;

use crate::svn::{text_diff, Diff, Item};
use crate::svnweb::Changed;

const CREATE_NO_WINDOW: u32 = 0x0800_0000;
// 차이를 보여 줄 파일 크기 상한 (svn 쪽과 같다)
const DIFF_MAX_BYTES: u64 = 3 * 1024 * 1024;

fn git(root: &Path, args: &[&str]) -> Result<Vec<u8>, String> {
    let out = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(["-c", "core.quotePath=false", "-c", "color.ui=never", "-c", "log.showSignature=false"])
        .args(args)
        .env("GIT_OPTIONAL_LOCKS", "0")
        .env("GIT_TERMINAL_PROMPT", "0")
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .map_err(|e| format!("git 을 실행하지 못했습니다. git 이 PATH 에 있어야 합니다 ({e})."))?;
    if !out.status.success() {
        let msg = String::from_utf8_lossy(&out.stderr).trim().to_string();
        return Err(if msg.is_empty() { format!("git {} 이 실패했습니다.", args[0]) } else { msg });
    }
    Ok(out.stdout)
}

fn git_text(root: &Path, args: &[&str]) -> Result<String, String> {
    git(root, args).map(|b| String::from_utf8_lossy(&b).into_owned())
}

/// 그 폴더가 git 저장소(또는 그 안)인가 — 설정의 「자동 감지」가 쓴다
pub fn is_repo(dir: &Path) -> bool {
    git_dir(dir).is_some()
}

/// .git 폴더. 작업 트리를 따로 둔 저장소(worktree)는 .git 이 「gitdir: 경로」 한 줄짜리 파일이다.
fn git_dir(root: &Path) -> Option<PathBuf> {
    let dot = root.join(".git");
    if dot.is_dir() {
        return Some(dot);
    }
    let line = fs::read_to_string(&dot).ok()?;
    let p = PathBuf::from(line.trim().strip_prefix("gitdir:")?.trim());
    Some(if p.is_absolute() { p } else { root.join(p) })
}

/// 저장소 변경을 알아채기 위한 표지 — 커밋 · 스테이지 · 브랜치 전환 · fetch 가 이 파일들 중 하나를 바꾼다
pub fn stamp(root: &Path) -> Option<u64> {
    let d = git_dir(root)?;
    ["index", "HEAD", "FETCH_HEAD"]
        .iter()
        .filter_map(|f| fs::metadata(d.join(f)).ok()?.modified().ok()?.duration_since(UNIX_EPOCH).ok())
        .map(|t| t.as_millis() as u64)
        .max()
}

fn toplevel(dir: &Path) -> Result<PathBuf, String> {
    let t = git_text(dir, &["rev-parse", "--show-toplevel"]).map_err(|e| format!("git 저장소가 아닙니다: {} ({e})", dir.display()))?;
    Ok(PathBuf::from(t.trim()))
}

/* ---------- 내 변경 ---------- */

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Status {
    pub root: String,             // 저장소 맨 위 폴더 — 경로는 모두 여기 기준
    pub branch: String,           // 브랜치 이름 (분리된 HEAD 면 빈 값)
    pub head: String,             // HEAD 커밋 앞 8자리
    pub upstream: Option<String>, // 원격 추적 브랜치 (예: origin/dev)
    pub ahead: u32,               // 아직 올리지 않은 내 커밋
    pub behind: u32,              // 마지막 fetch 기준으로 받지 않은 커밋
    pub took_ms: u64,
    pub items: Vec<Item>,
}

fn item(path: &str, code: &'static str, label: &'static str) -> Item {
    Item {
        path: path.to_string(),
        code,
        label,
        dir: path.ends_with('/'),
        conflict: code == "C",
        prop_changed: false,
        rev: None,
        author: None,
        date_ms: None,
        orig: None,
    }
}

// XY 두 글자(X 스테이지 · Y 작업 파일)를 svn 쪽과 같은 표시로 바꾼다. 스테이지 여부는 커밋 도우미가 git add 로 맞추므로 가리지 않는다.
fn code_of(xy: &str) -> (&'static str, &'static str) {
    let has = |c: char| xy.contains(c);
    if has('D') {
        ("D", "삭제")
    } else if has('A') {
        ("A", "추가")
    } else if has('R') || has('C') {
        ("M", "이름 바꿈")
    } else if has('T') {
        ("~", "종류 바뀜")
    } else {
        ("M", "수정")
    }
}

/// git status --porcelain=v2 를 읽는다. 이름을 바꾼 파일은 새 이름으로 보이고 옛 이름을 orig 에 단다.
pub fn status(dir: &Path) -> Result<Status, String> {
    let t0 = Instant::now();
    let root = toplevel(dir)?;
    let out = git_text(&root, &["status", "--porcelain=v2", "--branch", "-z", "--untracked-files=all"])?;
    let mut st = Status {
        root: root.to_string_lossy().replace('\\', "/"),
        branch: String::new(),
        head: String::new(),
        upstream: None,
        ahead: 0,
        behind: 0,
        took_ms: 0,
        items: Vec::new(),
    };
    let mut recs = out.split('\0');
    while let Some(r) = recs.next() {
        if let Some(h) = r.strip_prefix("# ") {
            let (k, v) = h.split_once(' ').unwrap_or((h, ""));
            match k {
                "branch.oid" => st.head = v.chars().take(8).collect(),
                "branch.head" if v != "(detached)" => st.branch = v.to_string(),
                "branch.upstream" => st.upstream = Some(v.to_string()),
                "branch.ab" => {
                    for n in v.split(' ') {
                        if let Some(a) = n.strip_prefix('+') {
                            st.ahead = a.parse().unwrap_or(0);
                        } else if let Some(b) = n.strip_prefix('-') {
                            st.behind = b.parse().unwrap_or(0);
                        }
                    }
                }
                _ => {}
            }
            continue;
        }
        let kind = r.chars().next().unwrap_or(' ');
        match kind {
            '1' => {
                let f: Vec<&str> = r.splitn(9, ' ').collect();
                if f.len() == 9 {
                    let (c, l) = code_of(f[1]);
                    st.items.push(item(f[8], c, l));
                }
            }
            '2' => {
                let f: Vec<&str> = r.splitn(10, ' ').collect();
                let orig = recs.next().map(str::to_string);
                if f.len() == 10 {
                    let (c, l) = code_of(f[1]);
                    st.items.push(Item { orig, ..item(f[9], c, l) });
                }
            }
            'u' => {
                let f: Vec<&str> = r.splitn(11, ' ').collect();
                if f.len() == 11 {
                    st.items.push(item(f[10], "C", "충돌"));
                }
            }
            '?' => st.items.push(item(&r[2..], "?", "추가 안 됨")),
            _ => {}
        }
    }
    st.items.sort_by(|a, b| a.path.cmp(&b.path));
    st.took_ms = t0.elapsed().as_millis() as u64;
    Ok(st)
}

// 어느 커밋의 파일 내용. 그 커밋에 없던 파일이면 None.
fn content(root: &Path, spec: &str, rel: &str) -> Option<Vec<u8>> {
    git(root, &["cat-file", "--filters", &format!("{spec}:{rel}")]).ok()
}

/// 마지막 커밋(HEAD)과 지금 파일의 차이. orig 는 이름을 바꾼 파일의 옛 경로 — HEAD 에서는 그 이름으로 찾는다.
pub fn diff(dir: &Path, rel: &str, orig: Option<&str>) -> Result<Diff, String> {
    let root = toplevel(dir)?;
    let full = root.join(rel);
    let mut out = Diff { path: rel.to_string(), note: None, added: 0, removed: 0, lines: Vec::new() };
    if full.is_dir() {
        out.note = Some("폴더입니다. 안의 파일을 고르면 차이를 보여 줍니다.".into());
        return Ok(out);
    }
    let old = content(&root, "HEAD", orig.unwrap_or(rel)).unwrap_or_default();
    let new = if full.exists() {
        if fs::metadata(&full).map(|m| m.len()).unwrap_or(0) > DIFF_MAX_BYTES {
            out.note = Some("파일이 커서(3MB 넘음) 차이를 보이지 않습니다.".into());
            return Ok(out);
        }
        fs::read(&full).map_err(|e| format!("파일을 읽지 못했습니다: {e}"))?
    } else {
        Vec::new()
    };
    let mut out = text_diff(rel, &old, &new);
    if out.lines.is_empty() && out.note.is_none() {
        out.note = Some("내용이 HEAD 와 같습니다 (수정 시각만 바뀜).".into());
    }
    Ok(out)
}

/* ---------- 로그 · 업데이트 확인 ---------- */

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Commit {
    pub id: String,             // 커밋 해시 전체
    pub short: String,          // 앞 8자리 (화면 표시)
    pub parent: Option<String>, // 첫 부모 — 없으면 첫 커밋
    pub author: String,
    pub date_ms: Option<i64>,
    pub msg: String,
    pub paths: Vec<Changed>,
}

// 커밋마다 \x1e 로 시작하고, 머리 칸은 \x1f 로 나눈다. 마지막 칸 뒤에 --name-status 줄이 붙는다.
const LOG_FORMAT: &str = "--format=%x1e%H%x1f%P%x1f%an%x1f%at%x1f%B%x1f";

fn parse_log(text: &str) -> Vec<Commit> {
    text.split('\x1e')
        .filter_map(|rec| {
            let f: Vec<&str> = rec.splitn(6, '\x1f').collect();
            if f.len() < 6 {
                return None;
            }
            let paths = f[5]
                .lines()
                .filter_map(|l| {
                    let cols: Vec<&str> = l.split('\t').collect();
                    if cols.len() < 2 {
                        return None;
                    }
                    let action = match cols[0].chars().next()? {
                        'A' | 'C' => "A",
                        'D' => "D",
                        'R' => "R",
                        _ => "M",
                    };
                    Some(Changed { action, path: cols[cols.len() - 1].to_string(), inside: true, dir: false })
                })
                .collect();
            Some(Commit {
                id: f[0].trim().to_string(),
                short: f[0].trim().chars().take(8).collect(),
                parent: f[1].split(' ').next().filter(|s| !s.is_empty()).map(str::to_string),
                author: f[2].to_string(),
                date_ms: f[3].trim().parse::<i64>().ok().map(|s| s * 1000),
                msg: f[4].trim_end().to_string(),
                paths,
            })
        })
        .collect()
}

/// git log — 최신부터 limit 개. rel 이 비면 저장소 전체, 아니면 그 파일 · 폴더.
pub fn log(dir: &Path, rel: &str, limit: u32) -> Result<Vec<Commit>, String> {
    let root = toplevel(dir)?;
    let n = format!("-n{limit}");
    let mut args = vec!["log", &n, LOG_FORMAT, "--name-status", "--no-renames"];
    if !rel.is_empty() {
        args.extend(["--", rel]);
    }
    Ok(parse_log(&git_text(&root, &args)?))
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Incoming {
    pub branch: String,
    pub head: String,
    pub upstream: Option<String>,
    pub fetched_ms: Option<u64>, // 마지막 fetch 시각 (.git/FETCH_HEAD) — 이보다 뒤의 원격 커밋은 보이지 않는다
    pub ahead: u32,
    pub entries: Vec<Commit>,
    pub overlap: Vec<String>, // 받을 커밋과 내가 고친 파일이 겹치는 경로 — 받으면 합치거나 충돌한다
    pub more: bool,
}

/// 받을 커밋 — HEAD..@{u} (원격에 묻지 않는다)
pub fn incoming(dir: &Path) -> Result<Incoming, String> {
    let st = status(dir)?;
    let root = PathBuf::from(&st.root);
    let fetched_ms = git_dir(&root)
        .and_then(|d| fs::metadata(d.join("FETCH_HEAD")).ok()?.modified().ok()?.duration_since(UNIX_EPOCH).ok())
        .map(|t| t.as_millis() as u64);
    let mut entries = Vec::new();
    if st.upstream.is_some() && st.behind > 0 {
        entries = parse_log(&git_text(&root, &["log", "-n200", LOG_FORMAT, "--name-status", "--no-renames", "HEAD..@{u}"])?);
    }
    let mine: std::collections::HashSet<&str> = st.items.iter().filter(|i| i.code != "?").map(|i| i.path.as_str()).collect();
    let mut overlap: Vec<String> = entries.iter().flat_map(|e| e.paths.iter()).filter(|c| mine.contains(c.path.as_str())).map(|c| c.path.clone()).collect();
    overlap.sort();
    overlap.dedup();
    let more = entries.len() >= 200;
    Ok(Incoming { branch: st.branch, head: st.head, upstream: st.upstream, fetched_ms, ahead: st.ahead, entries, overlap, more })
}

/// 두 내용의 차이. 쪽마다 「local」(지금 파일) · 「head」(원격 추적 브랜치, 없으면 HEAD) · 「none」(빈 내용) · 커밋 표기 중 하나
pub fn rev_diff(dir: &Path, rel: &str, old: &str, new: &str) -> Result<Diff, String> {
    let root = toplevel(dir)?;
    let get = |spec: &str| -> Result<Vec<u8>, String> {
        match spec {
            "local" => Ok(fs::read(root.join(rel)).unwrap_or_default()),
            "none" => Ok(Vec::new()),
            "head" => Ok(content(&root, "@{u}", rel).or_else(|| content(&root, "HEAD", rel)).unwrap_or_default()),
            // 「-」 로 시작하면 git 이 옵션으로 읽는다. 커밋 표기(해시 · 해시^)만 받는다.
            s if !s.is_empty() && s.chars().all(|c| c.is_ascii_hexdigit() || c == '^' || c == '~') => Ok(content(&root, s, rel).unwrap_or_default()),
            s => Err(format!("커밋 표기가 올바르지 않습니다: {s}")),
        }
    };
    Ok(text_diff(rel, &get(old)?, &get(new)?))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 로그를_커밋과_바뀐_파일로_나눈다() {
        let text = "\x1eabc123456789\x1fdef0\x1f홍길동\x1f1700000000\x1f첫 줄\n\n본문\n\x1f\n\nM\tsrc/a.java\nA\tsrc/새 파일.jsp\nD\tb.xml\n\
                    \x1e999\x1f\x1fkim\x1f1700000001\x1froot\n\x1f\n\nA\tREADME.md\n";
        let c = parse_log(text);
        assert_eq!(c.len(), 2);
        assert_eq!(c[0].short, "abc12345");
        assert_eq!(c[0].parent.as_deref(), Some("def0"));
        assert_eq!(c[0].msg, "첫 줄\n\n본문");
        assert_eq!(c[0].date_ms, Some(1_700_000_000_000));
        assert_eq!(c[0].paths.iter().map(|p| (p.action, p.path.as_str())).collect::<Vec<_>>(), vec![("M", "src/a.java"), ("A", "src/새 파일.jsp"), ("D", "b.xml")]);
        assert_eq!(c[1].parent, None);
    }

    #[test]
    fn 상태_두_글자를_표시로_바꾼다() {
        assert_eq!(code_of(".M").0, "M");
        assert_eq!(code_of("A.").0, "A");
        assert_eq!(code_of(".D").0, "D");
        assert_eq!(code_of("R.").1, "이름 바꿈");
    }

    #[test]
    #[ignore] // 실제 저장소로 — TOMCATRUN_TEST_GIT=<폴더> cargo test -- --ignored
    fn 실제_저장소를_읽기만_한다() {
        let Ok(dir) = std::env::var("TOMCATRUN_TEST_GIT") else { return };
        let dir = Path::new(&dir);
        let before = stamp(dir);
        let st = status(dir).unwrap();
        assert!(!st.head.is_empty());
        let _ = incoming(dir).unwrap();
        assert_eq!(before, stamp(dir), "읽기만 했으니 .git 이 바뀌지 않는다");
    }
}
