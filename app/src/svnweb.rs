// 읽기 전용 SVN 저장소 조회 — TortoiseSVN 의 「업데이트 확인 · 로그 보기 · 저장소 최신본과 비교」 몫.
//
// 저장소 서버(Apache mod_dav_svn)에 HTTP 로 읽기 요청만 보낸다.
//   OPTIONS  저장소 최신 리비전 (SVN-Youngest-Rev) · 리비전 경로 꼴 (SVN-Rev-Root-Stub)
//   REPORT   로그 (log-report · svn log 과 같은 것)
//   GET      어느 리비전의 파일 내용 (svn cat 과 같은 것)
// 업데이트 · 커밋처럼 저장소나 작업복사본을 바꾸는 요청은 보내지 않는다 (전역 규칙 · 화면은 명령을 복사만 한다).
//
// 저장소 주소는 작업복사본(.svn/wc.db)에서 읽는다 — 코드 · 설정에 적지 않는다.
// 서버가 Basic 인증을 요구하면 처음 한 번 화면에서 계정을 묻는다.
// 계정은 Windows 자격 증명 관리자에 「tomcatrun:svn:<호스트>」 로 둔다. 로그 · 화면 어디에도 비밀번호를 내지 않는다.

use std::io::Read;
use std::time::Duration;

use base64::Engine;
use serde::Serialize;

#[derive(Debug)]
pub enum WebErr {
    NeedLogin(String), // 계정이 없거나 틀림
    NotFound,          // 그 리비전 · 경로에 없음
    Other(String),
}

impl std::fmt::Display for WebErr {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            WebErr::NeedLogin(m) | WebErr::Other(m) => f.write_str(m),
            WebErr::NotFound => f.write_str("저장소에 그 경로가 없습니다."),
        }
    }
}

/* ---------- 계정 (Windows 자격 증명 관리자) ---------- */

pub fn cred_target(host: &str) -> String {
    format!("tomcatrun:svn:{host}")
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

pub fn cred_get(host: &str) -> Option<(String, String)> {
    cred_read(&cred_target(host))
}

fn cred_read(target: &str) -> Option<(String, String)> {
    use windows_sys::Win32::Security::Credentials::{CredFree, CredReadW, CREDENTIALW, CRED_TYPE_GENERIC};
    let t = wide(target);
    unsafe {
        let mut p: *mut CREDENTIALW = std::ptr::null_mut();
        if CredReadW(t.as_ptr(), CRED_TYPE_GENERIC, 0, &mut p) == 0 || p.is_null() {
            return None;
        }
        let c = &*p;
        let user = if c.UserName.is_null() {
            String::new()
        } else {
            let mut n = 0;
            while *c.UserName.add(n) != 0 {
                n += 1;
            }
            String::from_utf16_lossy(std::slice::from_raw_parts(c.UserName, n))
        };
        let blob = std::slice::from_raw_parts(c.CredentialBlob, c.CredentialBlobSize as usize);
        let pass = String::from_utf8_lossy(blob).into_owned();
        CredFree(p as *const _);
        Some((user, pass))
    }
}

pub fn cred_set(host: &str, user: &str, pass: &str) -> Result<(), String> {
    use windows_sys::Win32::Security::Credentials::{CredWriteW, CREDENTIALW, CRED_PERSIST_LOCAL_MACHINE, CRED_TYPE_GENERIC};
    let t = wide(&cred_target(host));
    let u = wide(user);
    let mut blob = pass.as_bytes().to_vec();
    unsafe {
        let mut c: CREDENTIALW = std::mem::zeroed();
        c.Type = CRED_TYPE_GENERIC;
        c.TargetName = t.as_ptr() as *mut u16;
        c.UserName = u.as_ptr() as *mut u16;
        c.CredentialBlobSize = blob.len() as u32;
        c.CredentialBlob = blob.as_mut_ptr();
        c.Persist = CRED_PERSIST_LOCAL_MACHINE;
        if CredWriteW(&c, 0) == 0 {
            return Err("자격 증명 관리자에 저장하지 못했습니다.".into());
        }
    }
    Ok(())
}

pub fn cred_delete(host: &str) {
    use windows_sys::Win32::Security::Credentials::{CredDeleteW, CRED_TYPE_GENERIC};
    let t = wide(&cred_target(host));
    unsafe {
        CredDeleteW(t.as_ptr(), CRED_TYPE_GENERIC, 0);
    }
}

/* ---------- 주소 ---------- */

pub fn host_of(url: &str) -> String {
    let rest = url.split("://").nth(1).unwrap_or(url);
    rest.split('/').next().unwrap_or("").to_string()
}

fn origin_of(url: &str) -> String {
    let scheme = url.split("://").next().unwrap_or("http");
    format!("{scheme}://{}", host_of(url))
}

// 경로 조각마다 퍼센트 부호화 (한글 · 공백 · # 등). / 는 그대로 둔다.
pub fn enc_path(p: &str) -> String {
    let mut out = String::new();
    for b in p.bytes() {
        if b.is_ascii_alphanumeric() || b"/-._~!$&'()*+,;=:@".contains(&b) {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

/* ---------- HTTP ---------- */

fn agent() -> ureq::Agent {
    ureq::AgentBuilder::new().timeout_connect(Duration::from_secs(5)).timeout(Duration::from_secs(30)).build()
}

fn auth(user: &str, pass: &str) -> String {
    format!("Basic {}", base64::engine::general_purpose::STANDARD.encode(format!("{user}:{pass}")))
}

fn call(method: &str, url: &str, cred: &(String, String), body: Option<&str>) -> Result<ureq::Response, WebErr> {
    let req = agent()
        .request(method, url)
        .set("Authorization", &auth(&cred.0, &cred.1))
        .set("Content-Type", "text/xml; charset=utf-8")
        .set("DAV", "http://subversion.tigris.org/xmlns/dav/svn/depth");
    let res = match body {
        Some(b) => req.send_string(b),
        None => req.call(),
    };
    match res {
        Ok(r) => Ok(r),
        Err(ureq::Error::Status(401, _)) | Err(ureq::Error::Status(403, _)) => {
            Err(WebErr::NeedLogin("SVN 계정이 없거나 맞지 않습니다.".into()))
        }
        Err(ureq::Error::Status(404, _)) => Err(WebErr::NotFound),
        Err(ureq::Error::Status(code, r)) => {
            let text = r.into_string().unwrap_or_default();
            let hint: String = text.chars().take(200).collect();
            Err(WebErr::Other(format!("저장소가 {code} 를 돌려주었습니다 ({method}). {hint}")))
        }
        Err(e) => Err(WebErr::Other(format!("저장소에 닿지 못했습니다: {e}"))),
    }
}

/// 저장소 최신 리비전과 리비전 경로 꼴
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Head {
    pub head: i64,
    pub rev_root_stub: Option<String>, // 예: /svn/repo/!svn/rvr (HTTPv2)
}

pub fn options(wc_url: &str, cred: &(String, String)) -> Result<Head, WebErr> {
    let body = r#"<?xml version="1.0" encoding="utf-8"?><D:options xmlns:D="DAV:"><D:activity-collection-set/></D:options>"#;
    let r = call("OPTIONS", wc_url, cred, Some(body))?;
    let head = r
        .header("SVN-Youngest-Rev")
        .and_then(|v| v.trim().parse().ok())
        .ok_or_else(|| WebErr::Other("저장소가 최신 리비전을 알려 주지 않습니다 (SVN-Youngest-Rev 없음).".into()))?;
    Ok(Head { head, rev_root_stub: r.header("SVN-Rev-Root-Stub").map(|s| s.to_string()) })
}

/// 어느 리비전의 파일 내용 (없으면 None — 그 리비전에 없던 파일)
pub fn cat(repo_root_url: &str, stub: &Option<String>, rev: i64, repos_path: &str, cred: &(String, String)) -> Result<Option<Vec<u8>>, WebErr> {
    let url = match stub {
        Some(s) => format!("{}{}/{rev}/{}", origin_of(repo_root_url), s, enc_path(repos_path)),
        // HTTPv2 를 모르는 서버 — 리비전 모음(baseline collection) 꼴
        None => format!("{}/!svn/bc/{rev}/{}", repo_root_url.trim_end_matches('/'), enc_path(repos_path)),
    };
    match call("GET", &url, cred, None) {
        Ok(r) => {
            let mut buf = Vec::new();
            r.into_reader().take(16 * 1024 * 1024).read_to_end(&mut buf).map_err(|e| WebErr::Other(e.to_string()))?;
            Ok(Some(buf))
        }
        Err(WebErr::NotFound) => Ok(None),
        Err(e) => Err(e),
    }
}

/* ---------- 로그 ---------- */

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Changed {
    pub action: &'static str, // A 추가 · M 수정 · D 삭제 · R 바꿈
    pub path: String,         // 작업복사본 기준 상대 경로 (작업복사본 밖이면 저장소 절대 경로)
    pub inside: bool,         // 작업복사본 안인가
    pub dir: bool,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LogEntry {
    pub rev: i64,
    pub author: String,
    pub date_ms: Option<i64>,
    pub msg: String,
    pub paths: Vec<Changed>,
}

/// svn log — start 에서 end 쪽으로 (start > end 면 최신부터). rel 은 작업복사본 안의 경로 (빈 값이면 전체).
pub fn log(wc_url: &str, wc_repos_path: &str, cred: &(String, String), start: i64, end: i64, limit: u32, rel: &str) -> Result<Vec<LogEntry>, WebErr> {
    let body = format!(
        r#"<?xml version="1.0" encoding="utf-8"?><S:log-report xmlns:S="svn:"><S:start-revision>{start}</S:start-revision><S:end-revision>{end}</S:end-revision><S:limit>{limit}</S:limit><S:discover-changed-paths/><S:revprop>svn:author</S:revprop><S:revprop>svn:date</S:revprop><S:revprop>svn:log</S:revprop><S:path>{}</S:path></S:log-report>"#,
        xml_escape(rel)
    );
    let r = call("REPORT", wc_url, cred, Some(&body))?;
    let mut text = String::new();
    r.into_reader().take(32 * 1024 * 1024).read_to_string(&mut text).map_err(|e| WebErr::Other(e.to_string()))?;
    parse_log(&text, wc_repos_path).map_err(WebErr::Other)
}

fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
}

pub fn parse_log(xml: &str, wc_repos_path: &str) -> Result<Vec<LogEntry>, String> {
    let doc = roxmltree::Document::parse(xml).map_err(|e| format!("로그 응답을 읽지 못했습니다: {e}"))?;
    let prefix = format!("/{}/", wc_repos_path.trim_matches('/'));
    let mut out = Vec::new();
    for item in doc.descendants().filter(|n| n.tag_name().name() == "log-item") {
        let mut e = LogEntry { rev: 0, author: String::new(), date_ms: None, msg: String::new(), paths: Vec::new() };
        for c in item.children().filter(|n| n.is_element()) {
            let text = c.text().unwrap_or("").to_string();
            match c.tag_name().name() {
                "version-name" => e.rev = text.trim().parse().unwrap_or(0),
                "creator-displayname" => e.author = text,
                "date" => e.date_ms = parse_date(&text),
                "comment" => e.msg = text,
                "added-path" | "modified-path" | "deleted-path" | "replaced-path" => {
                    let action = match c.tag_name().name() {
                        "added-path" => "A",
                        "deleted-path" => "D",
                        "replaced-path" => "R",
                        _ => "M",
                    };
                    let inside = text.starts_with(&prefix);
                    e.paths.push(Changed {
                        action,
                        path: if inside { text[prefix.len()..].to_string() } else { text.clone() },
                        inside,
                        dir: c.attribute("node-kind") == Some("dir"),
                    });
                }
                _ => {}
            }
        }
        e.paths.sort_by(|a, b| b.inside.cmp(&a.inside).then_with(|| a.path.cmp(&b.path)));
        out.push(e);
    }
    Ok(out)
}

// 「2026-10-27T05:56:21.123456Z」 → 밀리초 (UTC)
fn parse_date(s: &str) -> Option<i64> {
    let s = s.trim();
    let (date, time) = s.split_once('T')?;
    let mut d = date.split('-').map(|x| x.parse::<i64>().ok());
    let (y, m, day) = (d.next()??, d.next()??, d.next()??);
    let time = time.trim_end_matches('Z');
    let mut t = time.split(':');
    let (hh, mm): (i64, i64) = (t.next()?.parse().ok()?, t.next()?.parse().ok()?);
    let sec: f64 = t.next()?.parse().ok()?;
    // 1970-01-01 부터 날 수 (그레고리력)
    let (yy, mo) = if m <= 2 { (y - 1, m + 12) } else { (y, m) };
    let days = 365 * yy + yy / 4 - yy / 100 + yy / 400 + (153 * (mo - 3) + 2) / 5 + day - 719469;
    Some(((days * 86400 + hh * 3600 + mm * 60) as f64 * 1000.0 + sec * 1000.0) as i64)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 로그_응답을_읽는다() {
        let xml = r#"<?xml version="1.0" encoding="utf-8"?>
<S:log-report xmlns:S="svn:" xmlns:D="DAV:">
<S:log-item>
<D:version-name>3760</D:version-name>
<D:creator-displayname>홍길동</D:creator-displayname>
<S:date>2026-10-27T05:56:21.123456Z</S:date>
<D:comment>[주문] 목록 단계 표시 변경 - 넷에서 셋으로</D:comment>
<S:modified-path node-kind="file" text-mods="true" prop-mods="false">/trunk/shop/src/main/webapp/WEB-INF/views/order/orderList.jsp</S:modified-path>
<S:added-path node-kind="dir" text-mods="false" prop-mods="false">/trunk/shop/src/main/resources/db</S:added-path>
<S:deleted-path node-kind="file">/2025/other/x.txt</S:deleted-path>
</S:log-item>
</S:log-report>"#;
        let v = parse_log(xml, "trunk/shop").unwrap();
        assert_eq!(v.len(), 1);
        let e = &v[0];
        assert_eq!((e.rev, e.author.as_str()), (3760, "홍길동"));
        assert!(e.msg.starts_with("[주문]"));
        assert_eq!(e.date_ms, Some(1_793_080_581_123)); // 2026-10-27 05:56:21.123 UTC
        let p: Vec<(&str, &str, bool, bool)> = e.paths.iter().map(|c| (c.action, c.path.as_str(), c.inside, c.dir)).collect();
        assert_eq!(
            p,
            [
                ("A", "src/main/resources/db", true, true),
                ("M", "src/main/webapp/WEB-INF/views/order/orderList.jsp", true, false),
                ("D", "/2025/other/x.txt", false, false),
            ]
        );
    }

    // 실제 저장소에 빈 아이디로 읽기 요청(OPTIONS)을 하나 보내 「로그인 필요」 로 이어지는지 본다.
    // 인증을 요구하는 저장소 주소를 환경 변수로 줄 때만 돈다: TOMCATRUN_TEST_SVN_URL=<주소> cargo test --release -- --ignored
    #[test]
    #[ignore]
    fn 실제_저장소는_로그인을_요구한다() {
        let Ok(url) = std::env::var("TOMCATRUN_TEST_SVN_URL") else { return };
        match options(&url, &(String::new(), String::new())) {
            Err(WebErr::NeedLogin(_)) => {}
            Err(e) => panic!("로그인 필요가 아닌 오류: {e}"),
            Ok(h) => panic!("계정 없이 열렸다: r{}", h.head),
        }
    }

    #[test]
    fn 주소_조각() {
        assert_eq!(host_of("http://10.0.0.5/svn/repo"), "10.0.0.5");
        assert_eq!(origin_of("http://10.0.0.5/svn/repo"), "http://10.0.0.5");
        assert_eq!(host_of("https://svn.example.com:8443/repo"), "svn.example.com:8443");
        assert_eq!(enc_path("trunk/shop/MD/외부 API.md"), "trunk/shop/MD/%EC%99%B8%EB%B6%80%20API.md");
        // # [ ] { } 는 주소에서 뜻이 있어 부호화한다 (작업복사본에 「#global['file.tmp']}」 같은 폴더가 생기기도 한다)
        assert_eq!(enc_path("a/#global['x']}"), "a/%23global%5B'x'%5D%7D");
    }
}
