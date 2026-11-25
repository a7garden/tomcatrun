// 에이전트 제어 창구 (2026.11.25~) — Claude Code 같은 에이전트가 실행기를 부르는 로컬 HTTP.
//
// 실행기가 켜지면 127.0.0.1 의 빈 포트에 열고, 포트 · 토큰을 %LOCALAPPDATA%\tomcatrun\ctl.json 에 적는다.
// 부르는 쪽(skill/tomcatrun/tomcatrun.mjs)은 그 파일을 읽어 X-Tomcatrun-Token 머리에 토큰을 실어 보낸다.
//
// 열어 두는 것
//   GET  /status                     서버마다 상태 · 컴파일 · 재시작해야 반영되는 것 (가볍다 — 기다리며 1초마다 부른다)
//   GET  /checks?srv=                서버 점검 (「상태」 탭과 같은 줄)
//   POST /start · /stop · /restart · /compile?srv=[&full=1]
//   GET  /log?srv=&src=tomcat|app&lines=N[&prev=1][&since=ms]
//   GET  /svn · /svn/incoming · /git · /git/incoming?srv=      읽기만 한다
//
// 열지 않는 것 — Commit · Update · Revert · Resolve · Cleanup · Push 같은 버전 관리 쓰기와 설정 바꾸기.
//   작업복사본은 운영 저장소와 바로 이어져 있어 에이전트가 쓰면 되돌리기 어렵다. 사람이 화면에서 누른다.
//
// 토큰이 없거나 틀리면 403 · 브라우저가 보낸 요청(Origin 머리)도 403 — 같은 PC 의 웹 페이지가 톰캣을 끄지 못하게 한다.
// 에이전트가 시작 · 중지 · 재시작을 하면 실행기 로그와 알림에 남긴다 — 사람이 모르는 새 톰캣이 내려가지 않게.

use std::collections::VecDeque;
use std::hash::{BuildHasher, Hasher};
use std::io::{Read, Seek, SeekFrom, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;
use std::thread;
use std::time::Duration;

use serde_json::{json, Value};
use tauri::{AppHandle, Emitter};

use crate::{cfgs, log, now_ms, rt, rt_count, shared, LogLine, Status};

const TOKEN_HEADER: &str = "x-tomcatrun-token";
const RECENT_MAX: usize = 2000; // 실행기 · 컴파일 · 의존 jar 로그를 이만큼 들고 있는다 (톰캣 로그는 logs/console.log 에 다 있다)
const LINES_MAX: usize = 5000;
// 요청은 짧다(조회 · 단추 누르기). 응답을 읽지 않는 호출이 스레드를 쌓지 못하게 동시에 이만큼만 받는다
const CONN_MAX: usize = 8;
static CONNS: AtomicUsize = AtomicUsize::new(0);

/* ---------- 최근 로그 (톰캣 밖) ---------- */

// 컴파일 오류 · 「재시작해야 반영」 · 자바 교체 결과는 화면에만 가고 파일에는 남지 않는다. 에이전트가 볼 수 있게 여기 모은다.
static RECENT: Mutex<VecDeque<(u64, LogLine)>> = Mutex::new(VecDeque::new());

pub fn remember(l: &LogLine) {
    if l.src == "tomcat" {
        return;
    }
    let mut q = RECENT.lock().unwrap();
    if q.len() >= RECENT_MAX {
        q.pop_front();
    }
    q.push_back((now_ms(), l.clone()));
}

/* ---------- 창구 ---------- */

/// %LOCALAPPDATA%\tomcatrun\ctl.json — 실행기를 어디에 깔았든(설치본 · 개발 폴더) 같은 자리라 부르는 쪽이 찾는다
pub fn info_path() -> PathBuf {
    let base = std::env::var_os("LOCALAPPDATA").map(PathBuf::from).unwrap_or_else(std::env::temp_dir);
    base.join("tomcatrun").join("ctl.json")
}

// 운영체제가 무작위로 씨를 주는 RandomState 두 개 — 따로 난수 crate 를 들이지 않는다
fn new_token() -> String {
    let a = std::collections::hash_map::RandomState::new().build_hasher().finish();
    let b = std::collections::hash_map::RandomState::new().build_hasher().finish();
    format!("{a:016x}{b:016x}")
}

pub fn serve(app: AppHandle) {
    let listener = match TcpListener::bind("127.0.0.1:0") {
        Ok(l) => l,
        Err(e) => {
            log(&app, None, "app", "warn", format!("에이전트 제어 창구를 열지 못했습니다: {e}"));
            return;
        }
    };
    let port = listener.local_addr().map(|a| a.port()).unwrap_or(0);
    let token = new_token();
    let info = json!({
        "port": port,
        "token": token,
        "pid": std::process::id(),
        "exe": std::env::current_exe().map(|p| crate::p(&p)).unwrap_or_default(),
        "version": env!("CARGO_PKG_VERSION"),
        "startedAtMs": now_ms(),
    });
    // 임시 파일에 쓰고 바꿔 단다 — 부르는 쪽이 쓰다 만 파일을 읽지 않게
    let path = info_path();
    let tmp = path.with_extension("json.tmp");
    let _ = std::fs::create_dir_all(path.parent().unwrap());
    if let Err(e) = std::fs::write(&tmp, serde_json::to_string_pretty(&info).unwrap()).and_then(|_| std::fs::rename(&tmp, &path)) {
        log(&app, None, "app", "warn", format!("에이전트 제어 창구 정보를 쓰지 못했습니다 ({}): {e}", crate::p(&path)));
        return;
    }
    for conn in listener.incoming() {
        let Ok(stream) = conn else { continue };
        if CONNS.fetch_add(1, Ordering::SeqCst) >= CONN_MAX {
            CONNS.fetch_sub(1, Ordering::SeqCst);
            continue; // 닫는다 — 부르는 쪽은 연결 오류로 본다
        }
        let (a, t) = (app.clone(), token.clone());
        thread::spawn(move || {
            handle(a, stream, &t);
            CONNS.fetch_sub(1, Ordering::SeqCst);
        });
    }
}

/// 실행기가 끝날 때 — 남겨 두면 그 포트를 나중에 잡은 다른 프로그램에 토큰을 보낸다. 내가 적은 파일일 때만 지운다
pub fn forget() {
    let path = info_path();
    let mine = std::fs::read_to_string(&path)
        .ok()
        .and_then(|t| serde_json::from_str::<Value>(&t).ok())
        .map_or(false, |v| v["pid"].as_u64() == Some(std::process::id() as u64));
    if mine {
        let _ = std::fs::remove_file(path);
    }
}

/// 파일 끝의 n 줄 — console.log 는 기동마다 SQL 로그로 수십 MB 가 되므로 끝에서부터 블록으로 거꾸로 읽는다
fn tail_lines(file: &Path, n: usize) -> std::io::Result<Vec<String>> {
    const BLOCK: u64 = 64 * 1024;
    let mut f = std::fs::File::open(file)?;
    let len = f.seek(SeekFrom::End(0))?;
    let mut pos = len;
    let mut buf: Vec<u8> = Vec::new();
    // 끝 줄바꿈 하나는 줄로 세지 않으므로 n + 1 개를 넘게 모으면 멈춘다
    while pos > 0 && buf.iter().filter(|&&b| b == b'\n').count() <= n {
        let step = BLOCK.min(pos);
        pos -= step;
        f.seek(SeekFrom::Start(pos))?;
        let mut chunk = vec![0u8; step as usize];
        f.read_exact(&mut chunk)?;
        chunk.extend_from_slice(&buf);
        buf = chunk;
    }
    let text = String::from_utf8_lossy(&buf);
    let all: Vec<&str> = text.lines().collect();
    // 맨 앞 줄은 블록 경계에서 잘렸을 수 있다 — 파일 처음까지 읽지 않았으면 버린다
    let from = if pos > 0 { 1 } else { 0 };
    let all = &all[from.min(all.len())..];
    Ok(all[all.len().saturating_sub(n)..].iter().map(|s| s.to_string()).collect())
}

struct Req {
    method: String,
    path: String,
    query: Vec<(String, String)>,
    token: String,
    origin: bool,
}

impl Req {
    fn q(&self, k: &str) -> Option<&str> {
        self.query.iter().find(|(n, _)| n == k).map(|(_, v)| v.as_str())
    }
}

fn handle(app: AppHandle, mut s: TcpStream, token: &str) {
    let _ = s.set_read_timeout(Some(Duration::from_secs(5)));
    let _ = s.set_write_timeout(Some(Duration::from_secs(10)));
    let (code, body) = match read_req(&mut s) {
        Some(r) if r.origin || r.token != token => (403, json!({ "error": "토큰이 맞지 않습니다. ctl.json 을 다시 읽으세요." })),
        Some(r) => match route(&app, &r) {
            Ok(v) => (200, v),
            Err((c, e)) => (c, json!({ "error": e })),
        },
        None => (400, json!({ "error": "요청을 읽지 못했습니다." })),
    };
    let text = body.to_string();
    let reason = match code {
        200 => "OK",
        400 => "Bad Request",
        403 => "Forbidden",
        404 => "Not Found",
        409 => "Conflict",
        _ => "Error",
    };
    let head = format!("HTTP/1.1 {code} {reason}\r\nContent-Type: application/json; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", text.len());
    let _ = s.write_all(head.as_bytes());
    let _ = s.write_all(text.as_bytes());
}

// 머리까지만 읽는다 — 본문을 받는 요청은 없다 (값은 모두 쿼리로)
fn read_req(s: &mut TcpStream) -> Option<Req> {
    let mut buf = Vec::new();
    let mut chunk = [0u8; 2048];
    while !buf.windows(4).any(|w| w == b"\r\n\r\n") {
        let n = s.read(&mut chunk).ok()?;
        if n == 0 || buf.len() > 16 * 1024 {
            return None;
        }
        buf.extend_from_slice(&chunk[..n]);
    }
    parse_req(&String::from_utf8_lossy(&buf))
}

fn parse_req(text: &str) -> Option<Req> {
    let mut lines = text.split("\r\n");
    let mut first = lines.next()?.split(' ');
    let method = first.next()?.to_string();
    let target = first.next()?;
    let (path, qs) = target.split_once('?').unwrap_or((target, ""));
    let query = qs
        .split('&')
        .filter(|kv| !kv.is_empty())
        .map(|kv| {
            let (k, v) = kv.split_once('=').unwrap_or((kv, ""));
            (pct_decode(k), pct_decode(v))
        })
        .collect();
    let (mut token, mut origin) = (String::new(), false);
    for l in lines.take_while(|l| !l.is_empty()) {
        let Some((k, v)) = l.split_once(':') else { continue };
        let k = k.trim().to_ascii_lowercase();
        if k == TOKEN_HEADER {
            token = v.trim().to_string();
        } else if k == "origin" {
            origin = true;
        }
    }
    Some(Req { method, path: pct_decode(path), query, token, origin })
}

fn pct_decode(s: &str) -> String {
    // 바이트로 본다 — 문자열을 자르면 % 뒤가 한글일 때 글자 경계에서 멈춘다
    let hex = |c: u8| (c as char).to_digit(16).map(|d| d as u8);
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        match (b[i], b.get(i + 1).copied().and_then(hex), b.get(i + 2).copied().and_then(hex)) {
            (b'%', Some(h), Some(l)) => {
                out.push(h * 16 + l);
                i += 3;
                continue;
            }
            (b'+', _, _) => out.push(b' '),
            (c, _, _) => out.push(c),
        }
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

type Res = Result<Value, (u16, String)>;

fn bad(e: impl Into<String>) -> (u16, String) {
    (400, e.into())
}

fn route(app: &AppHandle, r: &Req) -> Res {
    let post = r.method == "POST";
    match (post, r.path.as_str()) {
        (false, "/status") => Ok(status(app)),
        (false, "/checks") => {
            let i = srv(app, r)?;
            let snap = crate::snapshot(app);
            Ok(json!({ "server": snap.servers[i].name, "checks": snap.servers[i].checks, "common": snap.checks }))
        }
        (true, "/start") | (true, "/stop") | (true, "/restart") => {
            let i = srv(app, r)?;
            let name = cfgs(app, i).1.name;
            let (gen, st) = {
                let x = rt(app, i).inner.lock().unwrap();
                (x.generation, x.status)
            };
            // 중지 · 재시작은 스레드로 넘기고 바로 돌아오므로 할 일이 없는 요청을 여기서 가린다 — 안 그러면 「요청했습니다」만 남는다
            if r.path == "/stop" && st == Status::Stopped {
                return Err((409, format!("{name} 톰캣은 이미 꺼져 있습니다.")));
            }
            if r.path == "/restart" && st == Status::Stopping {
                return Err((409, format!("{name} 톰캣을 끄는 중입니다. 다 꺼진 뒤 시작하세요.")));
            }
            let (what, res) = match r.path.as_str() {
                "/start" => ("시작", crate::start(app.clone(), i)),
                "/stop" => ("중지", crate::stop(app.clone(), i)),
                _ => ("재시작", crate::restart(app.clone(), i)),
            };
            res.map_err(|e| (409, e))?;
            // 결과가 아니라 요청을 남긴다 — 중지 · 재시작은 뒤에서 돌고, 실패하면 실행기가 따로 알린다
            let msg = format!("에이전트가 {name} 톰캣 {what}을 요청했습니다.");
            log(app, Some(i), "app", "", msg.clone());
            let _ = app.emit("toast", msg);
            // 부르는 쪽은 generation 이 이 값보다 커진 뒤(새 기동) running · stopped 를 기다린다
            Ok(json!({ "ok": true, "srv": i, "generation": gen, "atMs": now_ms() }))
        }
        (true, "/compile") => {
            let i = srv(app, r)?;
            let full = matches!(r.q("full"), Some("1") | Some("true"));
            crate::compile(app.clone(), i, full).map_err(|e| (409, e))?;
            Ok(json!({ "ok": true, "srv": i, "full": full, "atMs": now_ms() }))
        }
        (false, "/log") => {
            let i = srv(app, r)?;
            let lines = r.q("lines").and_then(|v| v.parse().ok()).unwrap_or(200usize).min(LINES_MAX);
            if r.q("src") == Some("app") {
                let since: u64 = r.q("since").and_then(|v| v.parse().ok()).unwrap_or(0);
                let q = RECENT.lock().unwrap();
                let all: Vec<Value> = q
                    .iter()
                    .filter(|(at, l)| *at >= since && (l.srv == Some(i) || l.srv.is_none()))
                    .map(|(at, l)| json!({ "atMs": at, "src": l.src, "kind": l.kind, "text": l.text }))
                    .collect();
                let skip = all.len().saturating_sub(lines);
                Ok(json!({ "lines": all[skip..] }))
            } else {
                let base = cfgs(app, i).1.catalina_base;
                let file = PathBuf::from(base).join("logs").join(if r.q("prev").is_some() { "console.prev.log" } else { "console.log" });
                let got = tail_lines(&file, lines).map_err(|e| (404, format!("{} 를 읽지 못했습니다: {e}", crate::p(&file))))?;
                // 부르는 쪽이 이번 기동의 로그인지 가린다 — 기동 전에 막히면 지난 기동의 파일이 그대로 남아 있다
                Ok(json!({ "file": crate::p(&file), "modifiedMs": crate::mtime_ms(&file), "lines": got }))
            }
        }
        (false, "/svn") => {
            let i = srv(app, r)?;
            block(crate::svn_status(app.clone(), i))
        }
        (false, "/svn/incoming") => {
            let i = srv(app, r)?;
            block(crate::svn_incoming(app.clone(), i))
        }
        (false, "/git") => {
            let i = srv(app, r)?;
            block(crate::git_status(app.clone(), i))
        }
        (false, "/git/incoming") => {
            let i = srv(app, r)?;
            block(crate::git_incoming(app.clone(), i))
        }
        _ => Err((404, format!("없는 명령: {} {}", r.method, r.path))),
    }
}

fn block<T: serde::Serialize>(f: impl std::future::Future<Output = Result<T, String>>) -> Res {
    let v = tauri::async_runtime::block_on(f).map_err(|e| (409, e))?;
    serde_json::to_value(v).map_err(|e| (500, e.to_string()))
}

// srv= 는 번호(0부터) · id · 이름 어느 것이든 받는다
fn srv(app: &AppHandle, r: &Req) -> Result<usize, (u16, String)> {
    let want = r.q("srv").ok_or_else(|| bad("srv= 로 서버를 정하세요 (번호 · id · 이름)"))?;
    let c = shared(app).cfg.lock().unwrap();
    let n = rt_count(app).min(c.servers.len());
    if let Ok(i) = want.parse::<usize>() {
        if i < n {
            return Ok(i);
        }
    }
    c.servers
        .iter()
        .take(n)
        .position(|s| s.id == want || s.name == want)
        .ok_or_else(|| (404, format!("없는 서버: {want} (있는 서버: {})", c.servers.iter().map(|s| format!("{} {}", s.id, s.name)).collect::<Vec<_>>().join(" · "))))
}

// 상태 잠금만 잠깐 쥔다 — 점검(파일 읽기)은 /checks 로 따로
fn status(app: &AppHandle) -> Value {
    let c = shared(app).cfg.lock().unwrap().clone();
    let n = rt_count(app).min(c.servers.len());
    let servers: Vec<Value> = (0..n)
        .map(|i| {
            let s = &c.servers[i];
            let r = rt(app, i);
            let vcs_busy = r.vcs_busy.load(std::sync::atomic::Ordering::Relaxed);
            let deps_busy = r.deps_busy.load(std::sync::atomic::Ordering::Relaxed);
            let x = r.inner.lock().unwrap();
            json!({
                "srv": i,
                "id": s.id,
                "name": s.name,
                "project": s.project,
                "url": format!("http://localhost:{}{}/", s.http_port, s.context_path),
                "httpPort": s.http_port,
                "status": x.status, // 화면과 같은 이름 (stopped · starting · running · stopping)
                "pid": x.child.as_ref().map(|ch| ch.id()),
                "generation": x.generation,
                "startedAtMs": x.started_at_ms,
                "startupMs": x.startup_ms,
                "waiting": x.waiting,
                "mode": x.mode,
                "compiling": x.compiling,
                "lastCompile": x.last_compile,
                // 컴파일한 자바 · 저장한 매퍼가 실행 중인 앱에 다 들어가기를 기다리는 중 — 끝나기 전에 화면을 눌러 보면 옛 동작이다
                "reflecting": x.java_wait.is_some() || x.mapper_wait.is_some(),
                "restartNeeded": x.restart_needed,
                "depsBusy": deps_busy,
                "vcsBusy": vcs_busy,
            })
        })
        .collect();
    json!({ "version": env!("CARGO_PKG_VERSION"), "autoCompile": c.auto_compile, "hotReload": c.hot_reload, "servers": servers })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 요청_머리에서_경로_쿼리_토큰을_읽는다() {
        let r = parse_req("POST /restart?srv=%EB%82%B4%EB%B6%80%EB%A7%9D&full=1 HTTP/1.1\r\nHost: x\r\nX-Tomcatrun-Token: abc\r\n\r\n").unwrap();
        assert_eq!(r.method, "POST");
        assert_eq!(r.path, "/restart");
        assert_eq!(r.q("srv"), Some("내부망"));
        assert_eq!(r.q("full"), Some("1"));
        assert_eq!(r.token, "abc");
        assert!(!r.origin);
    }

    #[test]
    fn 브라우저_요청을_가린다() {
        let r = parse_req("POST /stop?srv=0 HTTP/1.1\r\nOrigin: http://evil\r\nX-Tomcatrun-Token: abc\r\n\r\n").unwrap();
        assert!(r.origin);
    }

    #[test]
    fn 깨진_퍼센트는_그대로_둔다() {
        assert_eq!(pct_decode("a%2"), "a%2");
        assert_eq!(pct_decode("a%zz"), "a%zz");
        assert_eq!(pct_decode("a+b%2Fc"), "a b/c");
    }

    #[test]
    fn 파일_끝_줄만_읽는다() {
        let dir = std::env::temp_dir().join(format!("tomcatrun-ctl-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let f = dir.join("console.log");
        // 블록(64KB) 경계를 여러 번 넘도록 길게
        let body: String = (0..20000).map(|i| format!("줄 {i} {}\n", "x".repeat(20))).collect();
        std::fs::write(&f, &body).unwrap();
        let got = tail_lines(&f, 3).unwrap();
        assert_eq!(got, vec![format!("줄 19997 {}", "x".repeat(20)), format!("줄 19998 {}", "x".repeat(20)), format!("줄 19999 {}", "x".repeat(20))]);
        assert_eq!(tail_lines(&f, 20000).unwrap().len(), 20000);
        std::fs::write(&f, "하나\n둘").unwrap();
        assert_eq!(tail_lines(&f, 10).unwrap(), vec!["하나", "둘"]);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn 토큰은_매번_다르다() {
        assert_ne!(new_token(), new_token());
        assert_eq!(new_token().len(), 32);
    }
}
