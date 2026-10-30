// 자동 업데이트 — GitHub 릴리스의 최신 판을 보고, 새 판이면 화면에 알린다. 사용자가 누르면 받아서 설치 파일을 조용히 돌린다.
//
//   확인    GET api.github.com/repos/<REPO>/releases/latest → tag_name(v0.7.0) 을 이 실행 파일의 판과 견준다
//           지난 응답의 ETag 를 If-None-Match 로 붙인다 — 304 는 GitHub 비인증 한도(IP 당 시간 60회)를 쓰지 않는다
//           태그는 새 판인데 설치 파일이 아직 없으면(릴리스 빌드 중) Building — 실행기가 몇 분 뒤 다시 묻는다
//           한도에 걸리면(403 · 429) API 를 쓰지 않는 github.com 주소로 판을 알아낸다 (check_web) — 한 공인 IP 를 나눠 쓰는 사무실에서 걸린다
//   받기    설치 파일(tomcatrun-setup-<판>.exe)을 %TEMP% 에 받고 SHA-256 을 대조한다
//           (GitHub 가 자산마다 주는 digest, 없으면 같은 릴리스의 <설치 파일>.sha256) — 둘 다 없거나 다르면 돌리지 않는다
//           서명(<설치 파일>.sig)도 PUBKEY 로 검증한다 — 해시는 같은 릴리스에서 오므로 깨진 파일만 잡고, 릴리스째 바뀐 파일은 서명만 잡는다
//   설치    tomcatrun-setup.exe /S /D=<실행기 폴더> 를 띄우고 실행기는 톰캣을 끄고 닫힌다.
//           설치 파일이 exe 를 바꾼 뒤 실행기를 다시 띄운다 (tomcatrun.nsi 의 .onInstSuccess)
//
// 설치본(실행기 폴더에 uninstall.exe 가 있는 곳)에서만 설치까지 한다. 소스에서 띄운 실행기는 알리기만 한다.
// TLS 는 Windows 인증서 저장소(SChannel)를 쓴다 — 사내 프록시가 인증서를 바꿔 끼우는 망에서도 Windows 가 믿는 인증서면 통한다.
// HTTPS_PROXY 같은 환경 변수가 있으면 그 프록시를 거친다.

use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use base64::Engine;
use minisign_verify::{PublicKey, Signature};
use serde::Serialize;
use sha2::{Digest, Sha256};

pub const REPO: &str = "a7garden/tomcatrun";

// 설치 파일 서명의 공개 키. 비밀 키는 ~/.tauri/tomcatrun.key 와 저장소 비밀 TAURI_SIGNING_PRIVATE_KEY 에 있다.
// 비밀 키를 잃으면 이미 깔린 실행기는 새 판을 받지 못한다 — 이 값이 실행 파일 안에 박혀 있다
const PUBKEY: &str = "dW50cnVzdGVkIGNvbW1lbnQ6IG1pbmlzaWduIHB1YmxpYyBrZXk6IDRFNkIxNTU2MjdGRDY1REMKUldUY1pmMG5WaFZyVGpDU0JFSGV5UGVtZy92eW9wbTBLSm42WE9CdzlkRHRla0wzNm5BWGloMDEK";

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Release {
    pub version: String,   // 0.7.0
    pub page: String,      // 릴리스 페이지
    pub notes: String,     // 릴리스 본문 — CHANGELOG.md 의 그 판 절 (release.yml 이 옮긴다)
    pub asset: String,     // 설치 파일 주소
    pub asset_name: String,
    pub size: u64,
    #[serde(skip)]
    pub sha256: Option<String>,
    #[serde(skip)]
    pub sha256_url: Option<String>,
    #[serde(skip)]
    pub sig_url: Option<String>,
}

fn agent() -> Result<ureq::Agent, String> {
    let tls = native_tls::TlsConnector::new().map_err(|e| format!("TLS 를 준비하지 못했습니다: {e}"))?;
    Ok(ureq::AgentBuilder::new()
        .tls_connector(Arc::new(tls))
        .try_proxy_from_env(true)
        .timeout_connect(Duration::from_secs(10))
        .timeout_read(Duration::from_secs(60))
        .user_agent(concat!("tomcatrun/", env!("CARGO_PKG_VERSION")))
        .build())
}

/// 「0.10.2」 → [0, 10, 2]. 숫자가 아닌 꼬리(-beta 등)는 버린다.
fn parse_ver(v: &str) -> Vec<u64> {
    v.trim().trim_start_matches('v').split('.').map(|x| x.chars().take_while(char::is_ascii_digit).collect::<String>().parse().unwrap_or(0)).collect()
}

pub fn newer(remote: &str, local: &str) -> bool {
    let (mut a, mut b) = (parse_ver(remote), parse_ver(local));
    let n = a.len().max(b.len());
    a.resize(n, 0);
    b.resize(n, 0);
    a > b
}

/// 확인 결과
pub enum Latest {
    NotModified,      // 지난 확인과 같은 응답 (304) — 지난 결과를 그대로 쓴다
    Current,          // 이 판이 최신
    Building(String), // 새 판의 태그는 있는데 설치 파일이 아직 붙지 않았다
    Ready(Release),
}

/// 최신 릴리스와 응답의 ETag. etag 는 지난 응답의 것 — 주면 바뀌지 않았을 때 NotModified
pub fn check(etag: Option<&str>) -> Result<(Latest, Option<String>), String> {
    let url = format!("https://api.github.com/repos/{REPO}/releases/latest");
    let mut req = agent()?.get(&url).set("Accept", "application/vnd.github+json");
    if let Some(t) = etag {
        req = req.set("If-None-Match", t);
    }
    let resp = match req.call() {
        Ok(r) => r,
        Err(ureq::Error::Status(403 | 429, _)) => return Ok((check_web(env!("CARGO_PKG_VERSION"))?, None)),
        Err(e) => return Err(format!("업데이트를 확인하지 못했습니다: {e}")),
    };
    let tag = resp.header("ETag").map(String::from);
    if resp.status() == 304 {
        return Ok((Latest::NotModified, tag));
    }
    let text = resp.into_string().map_err(|e| e.to_string())?;
    Ok((classify(&text, env!("CARGO_PKG_VERSION"))?, tag))
}

/// API 한도에 걸렸을 때 — github.com/<REPO>/releases/latest 가 최신 태그 주소로 넘겨 주는 것으로 판을 알아낸다.
/// 릴리스 본문은 받지 못해 비워 둔다 (설정 › 업데이트의 변경 기록이 릴리스 페이지로 안내한다).
fn check_web(local: &str) -> Result<Latest, String> {
    let ag = agent()?;
    let resp = ag.get(&format!("https://github.com/{REPO}/releases/latest")).call().map_err(|e| format!("업데이트를 확인하지 못했습니다: {e}"))?;
    let Some(version) = version_in_url(resp.get_url()) else { return Ok(Latest::Current) };
    if !newer(&version, local) {
        return Ok(Latest::Current);
    }
    let base = format!("https://github.com/{REPO}/releases/download/v{version}");
    let asset_name = format!("tomcatrun-setup-{version}.exe");
    let asset = format!("{base}/{asset_name}");
    let size = match ag.head(&asset).call() {
        Ok(r) => r.header("Content-Length").and_then(|n| n.parse().ok()).unwrap_or(0),
        Err(ureq::Error::Status(404, _)) => return Ok(Latest::Building(version)),
        Err(e) => return Err(format!("업데이트를 확인하지 못했습니다: {e}")),
    };
    Ok(Latest::Ready(Release {
        page: format!("https://github.com/{REPO}/releases/tag/v{version}"),
        notes: String::new(),
        sha256: None,
        sha256_url: Some(format!("{asset}.sha256")),
        sig_url: Some(format!("{asset}.sig")),
        asset,
        asset_name,
        size,
        version,
    }))
}

/// 「…/releases/tag/v0.7.0」 → 0.7.0. 릴리스가 하나도 없으면 태그 주소로 넘어가지 않아 None
fn version_in_url(url: &str) -> Option<String> {
    let (_, tag) = url.split_once("/releases/tag/")?;
    let v = tag.trim_end_matches('/').trim_start_matches('v');
    (!v.is_empty()).then(|| v.to_string())
}

fn classify(text: &str, local: &str) -> Result<Latest, String> {
    if let Some(r) = parse_latest(text)? {
        return Ok(if newer(&r.version, local) { Latest::Ready(r) } else { Latest::Current });
    }
    // 설치 파일이 없는 릴리스 — 태그가 이 판보다 높으면 Actions 가 아직 설치 파일을 만드는 중이다
    let v: serde_json::Value = serde_json::from_str(text).map_err(|e| format!("릴리스 응답을 읽지 못했습니다: {e}"))?;
    let tag = v["tag_name"].as_str().unwrap_or("");
    let pre = v["draft"].as_bool() == Some(true) || v["prerelease"].as_bool() == Some(true);
    Ok(if !tag.is_empty() && !pre && newer(tag, local) { Latest::Building(tag.trim_start_matches('v').to_string()) } else { Latest::Current })
}

/// 실패 · 빌드 중일 때 다음 확인까지 (ms). 연달아 n 번째면 5 · 10 · 20 · 40 · 60분 — 1시간을 넘기지 않는다
pub fn retry_after(fails: u32) -> u64 {
    let min = 5u64 << fails.clamp(1, 5).saturating_sub(1);
    min.min(60) * 60_000
}

fn parse_latest(text: &str) -> Result<Option<Release>, String> {
    let v: serde_json::Value = serde_json::from_str(text).map_err(|e| format!("릴리스 응답을 읽지 못했습니다: {e}"))?;
    let tag = v["tag_name"].as_str().unwrap_or("");
    if tag.is_empty() || v["draft"].as_bool() == Some(true) || v["prerelease"].as_bool() == Some(true) {
        return Ok(None);
    }
    let assets = v["assets"].as_array().cloned().unwrap_or_default();
    let name_of = |a: &serde_json::Value| a["name"].as_str().unwrap_or("").to_string();
    let Some(exe) = assets.iter().find(|a| {
        let n = name_of(a).to_ascii_lowercase();
        n.starts_with("tomcatrun-setup-") && n.ends_with(".exe")
    }) else {
        return Ok(None);
    };
    let asset_name = name_of(exe);
    let sha256 = exe["digest"].as_str().and_then(|d| d.strip_prefix("sha256:")).map(|d| d.to_ascii_lowercase());
    let sha256_url = assets
        .iter()
        .find(|a| name_of(a).eq_ignore_ascii_case(&format!("{asset_name}.sha256")))
        .and_then(|a| a["browser_download_url"].as_str())
        .map(String::from);
    let sig_url = assets
        .iter()
        .find(|a| name_of(a).eq_ignore_ascii_case(&format!("{asset_name}.sig")))
        .and_then(|a| a["browser_download_url"].as_str())
        .map(String::from);
    let notes: String = v["body"].as_str().unwrap_or("").chars().take(20000).collect();
    Ok(Some(Release {
        version: tag.trim_start_matches('v').to_string(),
        page: v["html_url"].as_str().unwrap_or("").to_string(),
        notes,
        asset: exe["browser_download_url"].as_str().unwrap_or("").to_string(),
        asset_name,
        size: exe["size"].as_u64().unwrap_or(0),
        sha256,
        sha256_url,
        sig_url,
    }))
}

/// 서명 도구(tauri signer)가 내는 꼴 — 파일 내용이 minisign 글자를 base64 로 한 번 더 싼 것이다
fn unbase64(text: &str) -> Result<String, String> {
    let bytes = base64::engine::general_purpose::STANDARD.decode(text.trim()).map_err(|e| format!("서명을 읽지 못했습니다: {e}"))?;
    String::from_utf8(bytes).map_err(|e| format!("서명을 읽지 못했습니다: {e}"))
}

fn public_key() -> Result<PublicKey, String> {
    PublicKey::decode(&unbase64(PUBKEY)?).map_err(|e| format!("공개 키를 읽지 못했습니다: {e}"))
}

fn signature(text: &str) -> Result<Signature, String> {
    Signature::decode(&unbase64(text)?).map_err(|e| format!("서명을 읽지 못했습니다: {e}"))
}

/// 설치 파일을 받아 해시와 서명을 대조한다. progress(받은 바이트) 는 1MB 마다 부른다.
pub fn download(rel: &Release, mut progress: impl FnMut(u64)) -> Result<PathBuf, String> {
    let ag = agent()?;
    let want = match (&rel.sha256, &rel.sha256_url) {
        (Some(h), _) => h.clone(),
        (None, Some(u)) => {
            let t = ag.get(u).call().map_err(|e| format!("해시 파일을 받지 못했습니다: {e}"))?.into_string().map_err(|e| e.to_string())?;
            t.split_whitespace().next().unwrap_or("").to_ascii_lowercase()
        }
        (None, None) => return Err("릴리스에 설치 파일의 해시가 없어 받지 않았습니다.".into()),
    };
    if want.len() != 64 {
        return Err("릴리스의 해시가 올바르지 않습니다.".into());
    }
    let Some(sig_url) = &rel.sig_url else { return Err("릴리스에 설치 파일의 서명이 없어 받지 않았습니다.".into()) };
    let sig_text = ag.get(sig_url).call().map_err(|e| format!("서명 파일을 받지 못했습니다: {e}"))?.into_string().map_err(|e| e.to_string())?;
    let (key, sig) = (public_key()?, signature(&sig_text)?);
    let mut verifier = key.verify_stream(&sig).map_err(|e| format!("서명을 읽지 못했습니다: {e}"))?;
    let dir = std::env::temp_dir().join("tomcatrun-update");
    let _ = fs::create_dir_all(&dir);
    let out = dir.join(&rel.asset_name);
    let part = out.with_extension("part");
    let mut body = ag.get(&rel.asset).call().map_err(|e| format!("설치 파일을 받지 못했습니다: {e}"))?.into_reader();
    let mut f = File::create(&part).map_err(|e| format!("임시 파일을 만들지 못했습니다: {e}"))?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; 64 * 1024];
    let (mut got, mut next) = (0u64, 0u64);
    loop {
        let n = body.read(&mut buf).map_err(|e| format!("설치 파일을 받다 끊겼습니다: {e}"))?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
        verifier.update(&buf[..n]);
        f.write_all(&buf[..n]).map_err(|e| e.to_string())?;
        got += n as u64;
        if got >= next {
            progress(got);
            next = got + (1 << 20);
        }
    }
    drop(f);
    let have = format!("{:x}", hasher.finalize());
    if have != want {
        let _ = fs::remove_file(&part);
        return Err(format!("받은 설치 파일의 해시가 릴리스와 다릅니다 ({have}). 설치하지 않았습니다."));
    }
    if let Err(e) = verifier.finalize() {
        let _ = fs::remove_file(&part);
        return Err(format!("받은 설치 파일의 서명이 맞지 않습니다 ({e}). 설치하지 않았습니다."));
    }
    let _ = fs::remove_file(&out);
    fs::rename(&part, &out).map_err(|e| e.to_string())?;
    Ok(out)
}

/// 설치본인지 — 설치 파일이 실행기 폴더에 남기는 uninstall.exe 로 가린다
pub fn installable(home: &Path) -> bool {
    home.join("uninstall.exe").is_file()
}

/// 설치 파일을 조용히 돌린다. /D 는 따옴표 없이 맨 끝에 와야 한다 (NSIS 규칙).
pub fn run_installer(setup: &Path, home: &Path) -> Result<(), String> {
    std::process::Command::new(setup)
        .arg("/S")
        .arg(format!("/D={}", home.to_string_lossy().replace('/', "\\")))
        .spawn()
        .map(|_| ())
        .map_err(|e| format!("설치 파일을 띄우지 못했습니다: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    // 실제 릴리스를 읽고 설치 파일을 받아 해시를 대조한다 (GitHub 에 닿을 때만): cargo test --release -- --ignored
    #[test]
    #[ignore]
    fn 실제_릴리스를_받아_대조한다() {
        let text = agent().unwrap().get(&format!("https://api.github.com/repos/{REPO}/releases/latest")).call().unwrap().into_string().unwrap();
        let r = parse_latest(&text).unwrap().expect("설치 파일이 있는 릴리스");
        assert!(r.sha256.is_some() || r.sha256_url.is_some());
        let f = download(&r, |_| {}).unwrap();
        assert_eq!(std::fs::metadata(&f).unwrap().len(), r.size);
        let _ = std::fs::remove_file(f);
    }

    // API 를 거치지 않고 판과 설치 파일을 찾는다 (GitHub 에 닿을 때만)
    #[test]
    #[ignore]
    fn 한도에_걸리면_github_주소로_찾는다() {
        let Latest::Ready(r) = check_web("0.0.1").unwrap() else { panic!("설치 파일이 있는 릴리스") };
        assert!(r.size > 0 && r.asset.ends_with(".exe") && r.notes.is_empty());
    }

    #[test]
    fn 태그_주소에서_판을_읽는다() {
        assert_eq!(version_in_url("https://github.com/a/b/releases/tag/v0.7.0").as_deref(), Some("0.7.0"));
        assert_eq!(version_in_url("https://github.com/a/b/releases"), None);
    }

    // fixture 는 ~/.tauri/tomcatrun.key 로 서명한 시험 글자다
    #[test]
    fn 서명을_검증한다() {
        let sig = signature("dW50cnVzdGVkIGNvbW1lbnQ6IHNpZ25hdHVyZSBmcm9tIHRhdXJpIHNlY3JldCBrZXkKUlVUY1pmMG5WaFZyVHZaVFJlU01vZUZMdDVyYlhFdkoxTVJvbjR2YWFYNzl2emRYRU9FRmRMNDNDd0RlUFEwOU9PRlplWDBjTEhXNHVLM2xOeERFWml5ZjlISCtXTmEwdlF3PQp0cnVzdGVkIGNvbW1lbnQ6IHRpbWVzdGFtcDoxNzkzMzQ3OTI2CWZpbGU6Zml4dHVyZS50eHQKT080K1VzUmVaVmVNWWJvanJVQURVR0tIcmk0bEYzZlpmZE1wTTVuSHBFSDdlblk5ZEozKyswM3FnbnZlZUdPL2J2NkVqcSs2NUdRMXloRVFNVlRRQ3c9PQo=").unwrap();
        let key = public_key().unwrap();
        let check = |data: &[u8]| {
            let mut v = key.verify_stream(&sig).unwrap();
            v.update(data);
            v.finalize().is_ok()
        };
        assert!(check(b"tomcatrun signature fixture\n"));
        assert!(!check(b"tomcatrun signature fixturE\n"));
    }

    #[test]
    fn 판을_견준다() {
        assert!(newer("v0.7.0", "0.6.0"));
        assert!(newer("0.10.0", "0.9.9"));
        assert!(newer("1.0", "0.9.12"));
        assert!(!newer("v0.6.0", "0.6.0"));
        assert!(!newer("0.5.9", "0.6.0"));
        assert!(!newer("0.6.0-beta", "0.6.0"));
    }

    #[test]
    fn 릴리스_응답에서_설치_파일과_해시를_찾는다() {
        let text = r#"{"tag_name":"v0.7.0","html_url":"https://github.com/a/b/releases/tag/v0.7.0","body":"고친 것",
            "draft":false,"prerelease":false,"assets":[
              {"name":"tomcatrun-setup-0.7.0.exe.sha256","browser_download_url":"https://x/s.sha256","size":90},
              {"name":"tomcatrun-setup-0.7.0.exe.sig","browser_download_url":"https://x/s.sig","size":90},
              {"name":"tomcatrun-setup-0.7.0.exe","browser_download_url":"https://x/s.exe","size":123,
               "digest":"sha256:ABCDEF"}]}"#;
        let r = parse_latest(text).unwrap().unwrap();
        assert_eq!((r.version.as_str(), r.asset.as_str(), r.size), ("0.7.0", "https://x/s.exe", 123));
        assert_eq!(r.sha256.as_deref(), Some("abcdef"));
        assert_eq!(r.sha256_url.as_deref(), Some("https://x/s.sha256"));
        assert_eq!(r.sig_url.as_deref(), Some("https://x/s.sig"));
        // 미리 보기 판 · 설치 파일이 없는 릴리스는 알리지 않는다
        assert!(parse_latest(r#"{"tag_name":"v9.0.0","prerelease":true,"assets":[]}"#).unwrap().is_none());
        assert!(parse_latest(r#"{"tag_name":"v9.0.0","assets":[{"name":"src.zip"}]}"#).unwrap().is_none());
    }

    #[test]
    fn 새_판_빌드_중을_가린다() {
        let ready = r#"{"tag_name":"v0.7.0","assets":[{"name":"tomcatrun-setup-0.7.0.exe","digest":"sha256:ab"}]}"#;
        assert!(matches!(classify(ready, "0.6.0").unwrap(), Latest::Ready(r) if r.version == "0.7.0"));
        assert!(matches!(classify(ready, "0.7.0").unwrap(), Latest::Current));
        // 태그만 달리고 설치 파일이 아직 없다 — 새 판이면 빌드 중, 아니면 최신
        let bare = r#"{"tag_name":"v0.7.0","assets":[]}"#;
        assert!(matches!(classify(bare, "0.6.0").unwrap(), Latest::Building(v) if v == "0.7.0"));
        assert!(matches!(classify(bare, "0.7.0").unwrap(), Latest::Current));
        assert!(matches!(classify(r#"{"tag_name":"v0.7.0","prerelease":true,"assets":[]}"#, "0.6.0").unwrap(), Latest::Current));
    }

    #[test]
    fn 실패하면_점점_늦게_다시_묻는다() {
        let m = |f| retry_after(f) / 60_000;
        assert_eq!([m(0), m(1), m(2), m(3), m(4), m(5), m(9)], [5, 5, 10, 20, 40, 60, 60]);
    }
}
