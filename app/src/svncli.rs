// SVN 쓰기 · Blame — svn.exe 를 부른다 (2026.10.29~).
//
// 쓰기(commit · update · revert · add · delete · resolve · merge)는 「설정 › 버전 관리 › Allow write」를 켠 서버에서
// 사용자가 단추를 누를 때만 부른다. 그 검사는 부르는 쪽(main.rs 의 vcs_* 명령)이 한다. 이 파일은 명령을 만들고 돌릴 뿐이다.
// 상태 · 차이 계산은 지금처럼 svn.rs 가 wc.db 를 읽기 전용으로 읽는다.
//
// 부르는 규칙
//   늘 --non-interactive   계정 · 인증서 · 충돌을 묻느라 멈추지 않게 한다 (충돌은 postpone 으로 남는다)
//   메시지는 UTF-8 임시 파일(-F · --encoding utf-8)로 넘긴다. 인자로 넘기면 따옴표 · 줄바꿈이 깨진다.
//   경로는 인자로 넘긴다. Windows 의 svn 은 넓은 글자 인자를 UTF-8 로 받으므로 한글 경로가 그대로 간다.
//   commit 만은 경로를 UTF-8 임시 파일(--targets)로 넘긴다. 파일이 수백 개면 명령줄 상한(32767자)을 넘는다.
//   이름에 @ 가 있으면 끝에 @ 를 붙인다. 붙이지 않으면 svn 이 그 뒤를 리비전(peg)으로 읽는다.
//   계정은 먼저 svn 자신의 인증 캐시를 쓰고, 인증에 실패하면 실행기 계정을 --password-from-stdin 으로 넣어 한 번 더 부른다.
//   비밀번호는 명령줄 · 프로세스 목록 · svn 캐시(--no-auth-cache) 어디에도 남지 않는다. --password-from-stdin 은 svn 1.10 부터다.
//   실패하면 오류 번호로 무엇을 하면 되는지를 앞에 붙인다(explain). 잠금 · 낡은 작업복사본은 화면이 Cleanup · Update 단추를 내밀도록 표지를 단다.
//
// 목차 : 프로세스(run · gitwrite 도 쓴다) · svn.exe 찾기 · 판(probe · block_reason) · 명령 인자 · 오류 풀이(explain) · Svn(call · commit) · blame 읽기

use std::collections::HashMap;
use std::io::{Read, Write};
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Mutex;
use std::thread;
use std::time::{Duration, Instant};

use serde::Serialize;

use crate::svn::{decode, BlameLine};
use crate::svnweb::{parse_date, WebErr};

const CREATE_NO_WINDOW: u32 = 0x0800_0000;
// 큰 업데이트 · 커밋도 끝나도록 넉넉히 둔다. 중간에 끊으면 작업복사본이 잠겨 cleanup 이 필요해지므로 짧게 잡지 않는다.
const SVN_LIMIT: Duration = Duration::from_secs(600);

/// 화면이 알아보는 표지 — 오류 글 맨 앞에 붙는다. 잠금이면 Cleanup, 낡았으면 Update 단추를 함께 보인다.
pub const LOCKED: &str = "VCS_LOCKED|";
pub const STALE: &str = "VCS_STALE|";

/* ---------- 프로세스 ---------- */

pub struct Out {
    pub ok: bool,
    pub stdout: Vec<u8>,
    pub stderr: String,
}

/// 명령 하나를 창 없이 돌린다. limit 을 넘기면 끊는다. stdin 이 있으면 쓰고 바로 닫는다 (비밀번호 한 줄).
pub fn run(mut cmd: Command, stdin: Option<&str>, limit: Duration) -> Result<Out, String> {
    cmd.stdin(if stdin.is_some() { Stdio::piped() } else { Stdio::null() })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .creation_flags(CREATE_NO_WINDOW);
    let mut child = cmd.spawn().map_err(|e| format!("실행하지 못했습니다: {e}"))?;
    if let (Some(s), Some(mut w)) = (stdin, child.stdin.take()) {
        let _ = w.write_all(s.as_bytes());
    }
    let (mut so, mut se) = (child.stdout.take().unwrap(), child.stderr.take().unwrap());
    // 출력이 파이프를 채우면 자식이 멈추므로 따로 읽는다
    let t1 = thread::spawn(move || {
        let mut b = Vec::new();
        let _ = so.read_to_end(&mut b);
        b
    });
    let t2 = thread::spawn(move || {
        let mut b = Vec::new();
        let _ = se.read_to_end(&mut b);
        b
    });
    let began = Instant::now();
    let status = loop {
        if let Some(st) = child.try_wait().map_err(|e| e.to_string())? {
            break st;
        }
        if began.elapsed() > limit {
            let _ = child.kill();
            let _ = child.wait();
            return Err(format!("{}초 안에 끝나지 않아 멈췄습니다.", limit.as_secs()));
        }
        thread::sleep(Duration::from_millis(50));
    };
    Ok(Out { ok: status.success(), stdout: t1.join().unwrap_or_default(), stderr: decode(&t2.join().unwrap_or_default()).trim().to_string() })
}

/* ---------- svn.exe 찾기 · 판 ---------- */

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Probe {
    pub exe: String,
    pub version: String, // 예: 1.14.2
    pub major: u32,
    pub minor: u32,
}

/// 설정에 적은 경로 → PATH → TortoiseSVN · SlikSVN 설치 폴더 → 설정에서 받은 svn(svnget.rs) 순서
/// 받은 svn 을 맨 끝에 둔다 — 이미 쓰던 svn 이 있는 PC 에서는 찾은 svn 이 바뀌지 않는다
pub fn locate(cfg_exe: &str) -> Option<PathBuf> {
    let cfg = cfg_exe.trim();
    if !cfg.is_empty() {
        let p = PathBuf::from(cfg);
        return p.is_file().then_some(p);
    }
    let from_path = std::env::var_os("PATH").and_then(|v| std::env::split_paths(&v).map(|d| d.join("svn.exe")).find(|p| p.is_file()));
    from_path.or_else(|| {
        ["C:/Program Files/TortoiseSVN/bin/svn.exe", "C:/Program Files/SlikSvn/bin/svn.exe", "C:/Program Files (x86)/SlikSvn/bin/svn.exe"]
            .iter()
            .map(PathBuf::from)
            .chain([crate::svnget::dir().join("bin").join("svn.exe")])
            .find(|p| p.is_file())
    })
}

fn parse_version(s: &str) -> Option<(u32, u32)> {
    let mut it = s.trim().split(['.', '-', ' ']).map(|x| x.parse::<u32>().ok());
    Some((it.next()??, it.next()??))
}

// 화면 상태를 그릴 때마다 부르므로 svn --version 은 경로마다 한 번만 돈다. 못 찾았으면 30초 뒤에 다시 본다 (그 사이 설치했을 수 있다).
static PROBES: Mutex<Option<HashMap<String, (Instant, Result<Probe, String>)>>> = Mutex::new(None);

pub fn probe(cfg_exe: &str) -> Result<Probe, String> {
    let key = cfg_exe.trim().to_string();
    {
        let g = PROBES.lock().unwrap();
        if let Some((at, r)) = g.as_ref().and_then(|m| m.get(&key)) {
            if r.is_ok() || at.elapsed() < Duration::from_secs(30) {
                return r.clone();
            }
        }
    }
    let r = (|| -> Result<Probe, String> {
        let exe = locate(&key).ok_or_else(|| {
            if key.is_empty() {
                "svn.exe 를 찾지 못했습니다. 설정 › 버전 관리에서 「svn 받기」를 누르거나, TortoiseSVN 의 「command line client tools」 를 설치하거나, svn.exe 경로를 적어 주세요.".to_string()
            } else {
                format!("svn.exe 가 없습니다: {key}")
            }
        })?;
        let mut c = Command::new(&exe);
        c.args(["--version", "--quiet"]);
        let out = run(c, None, Duration::from_secs(10))?;
        let v = String::from_utf8_lossy(&out.stdout).trim().to_string();
        let (major, minor) = parse_version(&v).ok_or_else(|| format!("svn 판을 읽지 못했습니다: {v}"))?;
        Ok(Probe { exe: exe.to_string_lossy().replace('\\', "/"), version: v, major, minor })
    })();
    PROBES.lock().unwrap().get_or_insert_with(HashMap::new).insert(key, (Instant::now(), r.clone()));
    r
}

/// 찾은 결과를 잊는다 — svn 을 받은 직후에 부른다. 잊지 않으면 30초 동안 「찾지 못했습니다」가 남는다
pub fn forget() {
    *PROBES.lock().unwrap() = None;
}

/// 이 svn 으로 이 작업복사본에 써도 되는가 — 안 되면 그 까닭. svn upgrade 는 부르지 않는다 (형식을 바꾸면 다른 도구가 못 연다).
pub fn block_reason(p: &Probe, wc_format: Option<i64>) -> Option<String> {
    if (p.major, p.minor) < (1, 10) {
        return Some(format!("svn {} 은 옛 판입니다. 1.10 이상이 필요합니다.", p.version));
    }
    match wc_format {
        None => Some("작업복사본 형식을 읽지 못했습니다 (.svn/wc.db).".into()),
        Some(f) if f < 31 => Some(format!("작업복사본 형식({f})이 옛 판이라 svn upgrade 가 필요합니다. 실행기는 형식을 바꾸지 않습니다.")),
        Some(f) if f > 31 && (p.major, p.minor) < (1, 15) => Some(format!("작업복사본 형식({f})이 svn {} 보다 새 판입니다.", p.version)),
        _ => None,
    }
}

/* ---------- 오류 풀이 ---------- */

/// svn 오류를 「무엇을 하면 되는가」로 옮긴다. 원문의 오류 줄은 뒤에 남겨 오류 번호로 찾을 수 있게 한다.
/// 2026-10-29 : 잠금(E155004)이 원문 세 줄로만 떠서 무엇을 해야 할지 몰랐다.
pub fn explain(err: &str) -> String {
    let has = |ks: &[&str]| ks.iter().any(|k| err.contains(k));
    let (mark, why) = if has(&["E155004", "E155037", "E200033", "E200030"]) {
        (LOCKED, "작업복사본이 잠겨 있습니다. 다른 SVN 프로그램(Eclipse 의 SVN · TortoiseSVN)이 작업 중이면 끝나기를 기다리고, 아니면 끊긴 작업이 남긴 잠금이니 Cleanup 하세요.")
    } else if has(&["E155011", "E160024", "E160028", "E170004", "E160042"]) {
        (STALE, "저장소에 더 새 변경이 있어 Commit 이 거절됐습니다. 먼저 Update 한 뒤 다시 Commit 하세요.")
    } else if has(&["E155015"]) {
        ("", "Conflicted 가 남아 있습니다. Resolve 한 뒤 다시 하세요.")
    } else if has(&["E200009"]) {
        ("", "고른 경로만으로는 올릴 수 없습니다. 새로 더한 폴더 안의 파일이면 그 폴더도 함께 고르세요.")
    } else if has(&["E195022", "E160038", "E160039"]) {
        ("", "다른 사람이 이 파일에 lock 을 걸어 두었습니다. lock 을 건 사람에게 풀어 달라고 하세요.")
    } else if has(&["E165001"]) {
        ("", "저장소 규칙(pre-commit hook)이 거절했습니다. 아래 저장소의 말을 보고 고친 뒤 다시 하세요.")
    } else if has(&["E170013", "E175002", "E731001", "E730060", "E120108", "E230001"]) {
        ("", "저장소에 닿지 못했습니다. 네트워크 · VPN · 저장소 주소를 보세요.")
    } else {
        ("", "")
    };
    // 「svn: 」 머리를 떼고 줄을 잇는다. 같은 줄이 되풀이되면 한 번만
    let mut lines: Vec<String> = Vec::new();
    for l in err.lines().map(str::trim).filter(|l| !l.is_empty()) {
        let l = l.strip_prefix("svn: ").unwrap_or(l).to_string();
        if !lines.contains(&l) {
            lines.push(l);
        }
    }
    let raw = lines.join("
");
    if why.is_empty() { format!("{mark}{raw}") } else { format!("{mark}{why}
{raw}") }
}

/* ---------- 명령 ---------- */

// svn 이 계정 문제로 실패했다는 표지 — E170001 인증 필요 · E215004 넣어 볼 계정이 더 없음
fn auth_failed(err: &str) -> bool {
    ["E170001", "E215004"].iter().any(|k| err.contains(k))
}

// @ 가 있는 경로는 peg 리비전으로 읽히지 않게 끝에 @ 를 붙인다
fn arg_path(p: &str) -> String {
    if p.contains('@') { format!("{p}@") } else { p.to_string() }
}

fn with_paths(mut head: Vec<String>, paths: &[String]) -> Vec<String> {
    head.push("--".into());
    head.extend(paths.iter().map(|p| arg_path(p)));
    head
}

/// deep 이 아니면 --depth empty : 고른 경로만 올린다. 「.」 의 속성 변경을 고른 것이 작업복사본 전체 커밋이 되지 않게 한다.
/// 새로 더한 폴더의 안쪽은 부르는 쪽이 경로를 하나하나 넣는다.
/// 지운 폴더가 끼면 deep 으로 부른다 — empty 로는 svn 이 「안에 든 것이 있는 폴더는 지울 수 없다」(E200009)며 거절한다.
pub fn commit_args(msg_file: &Path, targets_file: &Path, deep: bool) -> Vec<String> {
    vec![
        "commit".into(),
        "--depth".into(),
        if deep { "infinity" } else { "empty" }.into(),
        "-F".into(),
        msg_file.to_string_lossy().into_owned(),
        "--encoding".into(),
        "utf-8".into(),
        "--targets".into(),
        targets_file.to_string_lossy().into_owned(),
    ]
}

/// 잠금 · 끊긴 작업만 푼다. 고친 파일을 되돌리거나(--revert) 버전 관리 밖 파일을 지우는(--remove-*) 옵션은 쓰지 않는다.
pub fn cleanup_args() -> Vec<String> {
    vec!["cleanup".into()]
}

pub fn update_args() -> Vec<String> {
    vec!["update".into(), "--accept".into(), "postpone".into()]
}

pub fn revert_args(deep: bool, paths: &[String]) -> Vec<String> {
    with_paths(vec!["revert".into(), "--depth".into(), if deep { "infinity" } else { "empty" }.into()], paths)
}

pub fn add_args(paths: &[String]) -> Vec<String> {
    with_paths(vec!["add".into(), "--depth".into(), "infinity".into()], paths)
}

pub fn delete_args(paths: &[String]) -> Vec<String> {
    with_paths(vec!["delete".into()], paths)
}

/// accept : mine(내 것으로) · theirs(저장소 것으로) · working(고친 파일 그대로)
pub fn resolve_args(accept: &str, path: &str) -> Result<Vec<String>, String> {
    let a = match accept {
        "mine" => "mine-full",
        "theirs" => "theirs-full",
        "working" => "working",
        x => return Err(format!("풀이 방법이 올바르지 않습니다: {x}")),
    };
    Ok(with_paths(vec!["resolve".into(), "--accept".into(), a.into()], &[path.to_string()]))
}

/// 리비전 하나의 변경을 거꾸로 작업복사본에 적용한다 (svn merge -c -N). 커밋은 따로 한다.
pub fn reverse_merge_args(rev: i64, wc_url: &str) -> Vec<String> {
    vec!["merge".into(), "-c".into(), format!("-{rev}"), "--accept".into(), "postpone".into(), wc_url.to_string(), ".".into()]
}

pub fn blame_args(path: &str) -> Vec<String> {
    with_paths(vec!["blame".into(), "--xml".into()], &[path.to_string()])
}

pub struct Svn {
    pub exe: PathBuf,
    pub wc: PathBuf,
}

impl Svn {
    fn once(&self, args: &[String], cred: Option<&(String, String)>) -> Result<Out, String> {
        let mut c = Command::new(&self.exe);
        c.current_dir(&self.wc).arg("--non-interactive");
        if let Some((u, _)) = cred {
            c.args(["--username", u.as_str(), "--password-from-stdin", "--no-auth-cache"]);
        }
        c.args(args);
        run(c, cred.map(|(_, p)| p.as_str()), SVN_LIMIT)
    }

    /// 먼저 svn 의 인증 캐시로, 계정 문제로 실패하면 실행기 계정으로 한 번 더. 그래도 안 되면 로그인 필요.
    pub fn call(&self, args: &[String], cred: Option<&(String, String)>) -> Result<String, WebErr> {
        // 시간이 다 돼 끊으면 svn 이 잡은 잠금이 남는다 — Cleanup 을 내민다
        let cut = |e: String| WebErr::Other(if e.contains("초 안에") { format!("{LOCKED}{e} 끊긴 svn 이 작업복사본에 잠금을 남겼을 수 있습니다. Cleanup 하세요.") } else { e });
        let mut out = self.once(args, None).map_err(cut)?;
        if !out.ok && auth_failed(&out.stderr) {
            let Some(c) = cred else { return Err(WebErr::NeedLogin("SVN 계정이 필요합니다.".into())) };
            out = self.once(args, Some(c)).map_err(cut)?;
            if !out.ok && auth_failed(&out.stderr) {
                return Err(WebErr::NeedLogin("SVN 계정이 없거나 맞지 않습니다.".into()));
            }
        }
        if !out.ok {
            let first = args.first().map(String::as_str).unwrap_or("");
            return Err(WebErr::Other(if out.stderr.is_empty() { format!("svn {first} 이 실패했습니다.") } else { explain(&out.stderr) }));
        }
        Ok(decode(&out.stdout))
    }

    /// 올린 리비전. 고른 파일 내용이 모두 BASE 와 같으면 svn 이 아무것도 올리지 않고 성공하므로 None 이다.
    pub fn commit(&self, msg: &str, paths: &[String], deep: bool, cred: Option<&(String, String)>) -> Result<Option<i64>, WebErr> {
        let file = temp_message(msg).map_err(WebErr::Other)?;
        let list: Vec<String> = paths.iter().map(|p| arg_path(p)).collect();
        let targets = match temp_file("targets", &list.join("
")) {
            Ok(t) => t,
            Err(e) => {
                let _ = std::fs::remove_file(&file);
                return Err(WebErr::Other(e));
            }
        };
        let r = self.call(&commit_args(&file, &targets, deep), cred);
        let _ = std::fs::remove_file(&file);
        let _ = std::fs::remove_file(&targets);
        Ok(committed_rev(&r?))
    }
}

fn temp_file(tag: &str, text: &str) -> Result<PathBuf, String> {
    let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
    let p = std::env::temp_dir().join(format!("tomcatrun-{tag}-{}-{nanos}.txt", std::process::id()));
    std::fs::write(&p, text.as_bytes()).map_err(|e| format!("임시 파일을 쓰지 못했습니다: {e}"))?;
    Ok(p)
}

pub fn temp_message(text: &str) -> Result<PathBuf, String> {
    temp_file("msg", text)
}

// 올린 것이 없으면 svn 은 아무 줄도 내지 않는다 — 그때는 None
fn committed_rev(out: &str) -> Option<i64> {
    if out.trim().is_empty() {
        return None;
    }
    last_number(out)
}

// 「Committed revision 3761.」 — 판 · 언어마다 문구가 달라 마지막 줄의 숫자만 읽는다
fn last_number(out: &str) -> Option<i64> {
    let line = out.lines().rev().find(|l| l.chars().any(|c| c.is_ascii_digit()))?;
    let digits: String = line.chars().rev().skip_while(|c| !c.is_ascii_digit()).take_while(|c| c.is_ascii_digit()).collect();
    digits.chars().rev().collect::<String>().parse().ok()
}

/// svn blame --xml — 줄마다 마지막으로 고친 리비전 · 작성자 · 시각 (줄 순서대로)
pub fn parse_blame(xml: &str) -> Result<Vec<(String, String, Option<i64>)>, String> {
    let doc = roxmltree::Document::parse(xml).map_err(|e| format!("blame 응답을 읽지 못했습니다: {e}"))?;
    let mut out = Vec::new();
    for e in doc.descendants().filter(|n| n.has_tag_name("entry")) {
        let c = e.children().find(|n| n.has_tag_name("commit"));
        let rev = c.and_then(|c| c.attribute("revision")).unwrap_or("").to_string();
        let text = |name: &str| c.and_then(|c| c.children().find(|n| n.has_tag_name(name))).and_then(|n| n.text()).unwrap_or("").to_string();
        out.push((rev, text("author"), parse_date(&text("date"))));
    }
    Ok(out)
}

/// 기준본(BASE) 내용과 blame 결과를 줄끼리 잇는다 — svn blame 은 작업복사본 경로면 BASE 를 본다
pub fn join_blame(base: &str, who: Vec<(String, String, Option<i64>)>) -> Vec<BlameLine> {
    base.lines()
        .zip(who.into_iter().chain(std::iter::repeat((String::new(), String::new(), None))))
        .map(|(s, (rev, author, date_ms))| BlameLine { rev, author, date_ms, s: s.trim_end_matches('\r').to_string() })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 명령_인자를_만든다() {
        let paths = vec!["src/주문.java".to_string(), "a@b.txt".to_string()];
        let c = commit_args(Path::new("C:/t/m.txt"), Path::new("C:/t/l.txt"), false);
        assert_eq!(c, ["commit", "--depth", "empty", "-F", "C:/t/m.txt", "--encoding", "utf-8", "--targets", "C:/t/l.txt"]);
        assert_eq!(commit_args(Path::new("m"), Path::new("l"), true)[2], "infinity");
        assert_eq!(arg_path(&paths[1]), "a@b.txt@");
        assert_eq!(cleanup_args(), ["cleanup"]);
        assert_eq!(revert_args(false, &paths[..1]), ["revert", "--depth", "empty", "--", "src/주문.java"]);
        assert_eq!(revert_args(true, &paths[..1])[2], "infinity");
        assert_eq!(resolve_args("theirs", "x").unwrap(), ["resolve", "--accept", "theirs-full", "--", "x"]);
        assert!(resolve_args("base", "x").is_err());
        assert_eq!(reverse_merge_args(3760, "http://h/svn/r/trunk")[2], "-3760");
        // 「-」 로 시작하는 이름도 옵션으로 읽히지 않는다
        assert_eq!(delete_args(&["-x".to_string()]), ["delete", "--", "-x"]);
    }

    #[test]
    fn 판과_형식을_가린다() {
        let p = |v: &str| {
            let (major, minor) = parse_version(v).unwrap();
            Probe { exe: String::new(), version: v.into(), major, minor }
        };
        assert_eq!(parse_version("1.14.2\r\n"), Some((1, 14)));
        assert!(block_reason(&p("1.14.2"), Some(31)).is_none());
        assert!(block_reason(&p("1.9.7"), Some(31)).is_some());
        assert!(block_reason(&p("1.14.2"), Some(29)).is_some());
        assert!(block_reason(&p("1.14.2"), Some(32)).is_some());
        assert!(block_reason(&p("1.15.0"), Some(32)).is_none());
    }

    #[test]
    fn 결과_줄을_읽는다() {
        assert_eq!(last_number("Sending        a.java\nTransmitting file data .done\nCommitting transaction...\nCommitted revision 3761.\n"), Some(3761));
        assert_eq!(last_number("리비전 3762 를 커밋했습니다.\r\n"), Some(3762));
        assert_eq!(last_number("nothing"), None);
        assert_eq!(committed_rev(""), None);
        assert_eq!(committed_rev("Committed revision 9.\n"), Some(9));
        assert!(auth_failed("svn: E170001: Authentication required for '<http://h:80> repo'"));
        assert!(!auth_failed("svn: E155015: Commit failed (details follow):"));
    }

    #[test]
    fn 오류를_풀어_쓴다() {
        let lock = "svn: E155004: Run 'svn cleanup' to remove locks (type 'svn help cleanup' for details)\nsvn: E155004: Commit failed (details follow):\nsvn: E155004: Working copy 'D:\\w\\src\\main' locked.\n";
        let e = explain(lock);
        assert!(e.starts_with(LOCKED) && e.contains("Cleanup") && e.contains("E155004: Working copy"));
        assert!(!e.contains("svn: E"));
        assert!(explain("svn: E160028: File '/trunk/a.java' is out of date").starts_with(STALE));
        assert!(explain("svn: E170013: Unable to connect").contains("네트워크"));
        assert_eq!(explain("svn: E999999: 낯선 오류"), "E999999: 낯선 오류");
    }

    #[test]
    fn blame_응답을_읽는다() {
        let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<blame><target path="a.jsp">
<entry line-number="1"><commit revision="3760"><author>홍길동</author><date>2026-10-27T05:56:21.123456Z</date></commit></entry>
<entry line-number="2"><commit revision="3512"><author>kim</author><date>2026-09-01T00:00:00.000000Z</date></commit></entry>
</target></blame>"#;
        let v = parse_blame(xml).unwrap();
        assert_eq!(v.len(), 2);
        assert_eq!((v[0].0.as_str(), v[0].1.as_str(), v[0].2), ("3760", "홍길동", Some(1_793_080_581_123)));
        let lines = join_blame("<c:if>\r\n</c:if>\n남은 줄\n", v);
        assert_eq!(lines.len(), 3);
        assert_eq!((lines[0].s.as_str(), lines[1].rev.as_str(), lines[2].rev.as_str()), ("<c:if>", "3512", ""));
    }
}
