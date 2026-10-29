// svn 받기 — svn.exe 를 못 찾은 PC 에서 「설정 › 버전 관리 › svn 받기」를 누르면 Subversion 명령줄 도구를 받아 푼다 (2026.10.29~).
//
//   받는 곳  VisualSVN 이 내는 Apache Subversion 명령줄 도구 zip (설치 없이 쓰는 꼴 · bin/svn.exe 와 DLL · VC 런타임까지 들어 있다)
//   대조     zip 의 SHA-256 을 아래 상수와 견준다. 다르면 풀지 않는다 — 주소가 같아도 내용이 바뀌었으면 쓰지 않는다
//   놓는 곳  %LOCALAPPDATA%/tomcatrun/svn — 실행기 폴더는 Program Files 라 권한이 모자랄 수 있고, 업데이트 설치 파일이 덮어쓰는 자리다
//            svn.new 에 다 풀고 bin/svn.exe 가 있는지 본 뒤에 svn 과 바꾼다. 중간에 끊겨도 반쯤 풀린 폴더를 쓰지 않는다
//
// 찾는 순서는 svncli::locate 가 정한다 — 설정 경로 → PATH → TortoiseSVN · SlikSVN → 여기서 받은 svn (맨 끝).
// TLS · 프록시는 업데이트 받기(update.rs)와 같다.

use std::fs::{self, File};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use sha2::{Digest, Sha256};

// 2026.10.29 : 1.15.0 — 판을 올릴 때는 세 줄을 함께 고친다 (VisualSVN 이 옛 판 zip 을 내리면 주소가 404 가 된다)
pub const VERSION: &str = "1.15.0";
const URL: &str = "https://www.visualsvn.com/files/Apache-Subversion-1.15.0-x64.zip";
const SHA256: &str = "98bb798dd6c70cc875b9bdacd600594351cf88f0a39a831a5055df061c871bb5";

/// 받은 svn 을 두는 폴더 — 그 아래 bin/svn.exe
pub fn dir() -> PathBuf {
    std::env::var_os("LOCALAPPDATA").map(PathBuf::from).unwrap_or_else(std::env::temp_dir).join("tomcatrun").join("svn")
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

/// 받아서 대조하고 푼다. progress(받은 바이트) 는 1MB 마다 부른다. 돌려주는 것은 svn.exe 경로.
pub fn install(mut progress: impl FnMut(u64)) -> Result<PathBuf, String> {
    let dest = dir();
    let base = dest.parent().unwrap().to_path_buf();
    fs::create_dir_all(&base).map_err(|e| format!("폴더를 만들지 못했습니다 ({}): {e}", base.display()))?;
    let zip_path = base.join("svn.zip.part");
    let mut body = agent()?.get(URL).call().map_err(|e| format!("svn 을 받지 못했습니다: {e}"))?.into_reader();
    let mut f = File::create(&zip_path).map_err(|e| format!("임시 파일을 만들지 못했습니다: {e}"))?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; 64 * 1024];
    let (mut got, mut next) = (0u64, 0u64);
    loop {
        let n = body.read(&mut buf).map_err(|e| format!("svn 을 받다 끊겼습니다: {e}"))?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
        f.write_all(&buf[..n]).map_err(|e| e.to_string())?;
        got += n as u64;
        if got >= next {
            progress(got);
            next = got + (1 << 20);
        }
    }
    drop(f);
    let have = format!("{:x}", hasher.finalize());
    if have != SHA256 {
        let _ = fs::remove_file(&zip_path);
        return Err(format!("받은 zip 의 해시가 다릅니다 ({have}). 풀지 않았습니다."));
    }
    let r = unpack(&zip_path, &dest);
    let _ = fs::remove_file(&zip_path);
    r?;
    Ok(dest.join("bin").join("svn.exe"))
}

// svn.new 에 다 푼 뒤 svn 과 바꾼다
fn unpack(zip_path: &Path, dest: &Path) -> Result<(), String> {
    let tmp = dest.with_file_name("svn.new");
    let _ = fs::remove_dir_all(&tmp);
    let r = extract(zip_path, &tmp);
    if let Err(e) = r {
        let _ = fs::remove_dir_all(&tmp);
        return Err(format!("zip 을 풀지 못했습니다: {e}"));
    }
    if !tmp.join("bin").join("svn.exe").is_file() {
        let _ = fs::remove_dir_all(&tmp);
        return Err("받은 zip 에 bin/svn.exe 가 없습니다.".into());
    }
    if dest.exists() {
        fs::remove_dir_all(dest).map_err(|e| format!("전에 받은 svn 을 지우지 못했습니다 (쓰는 중일 수 있습니다): {e}"))?;
    }
    fs::rename(&tmp, dest).map_err(|e| format!("받은 svn 을 옮기지 못했습니다: {e}"))
}

fn extract(zip_path: &Path, to: &Path) -> io::Result<()> {
    let mut z = zip::ZipArchive::new(File::open(zip_path)?).map_err(io::Error::other)?;
    for i in 0..z.len() {
        let mut e = z.by_index(i).map_err(io::Error::other)?;
        // 풀 폴더 밖을 가리키는 이름(../ · 절대 경로)은 건너뛴다. 이 zip 은 이름을 \ 로 나누므로 / 로 바꿔 본다
        let name = e.name().replace('\\', "/");
        let Some(rel) = safe_rel(&name) else { continue };
        let out = to.join(rel);
        if name.ends_with('/') {
            fs::create_dir_all(&out)?;
            continue;
        }
        if let Some(p) = out.parent() {
            fs::create_dir_all(p)?;
        }
        io::copy(&mut e, &mut File::create(&out)?)?;
    }
    Ok(())
}

// 「bin/svn.exe」 → bin/svn.exe. 빈 이름 · 「..」 · 드라이브 · 맨 앞 / 가 있으면 None
fn safe_rel(name: &str) -> Option<PathBuf> {
    let parts: Vec<&str> = name.split('/').filter(|s| !s.is_empty() && *s != ".").collect();
    if parts.is_empty() || name.starts_with('/') || parts.iter().any(|s| *s == ".." || s.contains(':')) {
        return None;
    }
    Some(parts.iter().collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 풀_폴더_밖은_건너뛴다() {
        assert_eq!(safe_rel("bin/svn.exe"), Some(PathBuf::from("bin").join("svn.exe")));
        assert_eq!(safe_rel("Licenses/zlib License.txt"), Some(PathBuf::from("Licenses").join("zlib License.txt")));
        assert_eq!(safe_rel("bin/"), Some(PathBuf::from("bin")));
        assert_eq!(safe_rel("../evil.dll"), None);
        assert_eq!(safe_rel("bin/../../evil.dll"), None);
        assert_eq!(safe_rel("/abs.dll"), None);
        assert_eq!(safe_rel("C:/abs.dll"), None);
        assert_eq!(safe_rel(""), None);
    }

    #[test]
    fn 해시는_소문자_64자() {
        assert_eq!(SHA256.len(), 64);
        assert!(SHA256.chars().all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()));
        assert!(URL.contains(VERSION));
    }

    // 망에 닿아야 하므로 따로 돌린다: cargo test 실제_zip_을_받아_풀고_svn_을_부른다 -- --ignored
    // LOCALAPPDATA 를 임시 폴더로 바꿔 사용자 폴더에 남기지 않는다
    #[test]
    #[ignore]
    fn 실제_zip_을_받아_풀고_svn_을_부른다() {
        let tmp = std::env::temp_dir().join("tomcatrun-svnget-test");
        let _ = fs::remove_dir_all(&tmp);
        std::env::set_var("LOCALAPPDATA", &tmp);
        let exe = install(|_| {}).unwrap();
        let out = std::process::Command::new(&exe).args(["--version", "--quiet"]).output().unwrap();
        assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), VERSION);
        assert!(!tmp.join("tomcatrun").join("svn.new").exists() && !tmp.join("tomcatrun").join("svn.zip.part").exists());
        // 다시 받으면 전에 받은 것을 갈아끼운다
        install(|_| {}).unwrap();
        let _ = fs::remove_dir_all(&tmp);
    }

    #[test]
    fn 놓는_곳은_tomcatrun_svn() {
        let d = dir();
        assert!(d.ends_with(Path::new("tomcatrun").join("svn")));
    }
}
