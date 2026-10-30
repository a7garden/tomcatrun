// 복사본 — 쓰기가 덮거나 지울 내 파일을 먼저 사용자 폴더에 남겨, 실행기 안에서 되살린다 (2026.10.30~)
//
// 놓는 곳  %LOCALAPPDATA%/tomcatrun/vcs-backup/<시각>-<번호>/  manifest.json · files/<경로>
// 남기는 기간  14일 — 새 복사본을 만들 때 지난 것을 지운다
// 복사본을 못 만들면 쓰기를 멈춘다. 파일 하나가 100MB 넘으면 빼고, 뺀 것을 결과에 적는다.
// 저장소 · 작업복사본의 .svn · .git 은 건드리지 않고 복사하지도 않는다.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

const KEEP: Duration = Duration::from_secs(14 * 24 * 3600);
const FILE_MAX: u64 = 100 * 1024 * 1024;

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Saved {
    pub id: String,
    pub what: String,         // Revert · Delete · Update …
    pub root: String,         // 작업 폴더 — 다른 서버의 복사본을 섞지 않는다
    pub when_ms: u64,
    pub files: Vec<String>,   // 남긴 파일 (root 기준)
    pub skipped: Vec<String>, // 너무 커서 뺀 파일
}

pub fn base_dir() -> PathBuf {
    std::env::var_os("LOCALAPPDATA").map(PathBuf::from).unwrap_or_else(std::env::temp_dir).join("tomcatrun").join("vcs-backup")
}

fn slash(p: &Path) -> String {
    p.to_string_lossy().replace('\\', "/")
}

fn safe_rel(rel: &str) -> bool {
    !rel.is_empty() && rel != "." && !rel.contains("..") && !Path::new(rel).is_absolute()
}

// 폴더면 안의 파일까지 (.svn · .git 빼고). 없는 경로는 남길 것이 없다.
fn walk(root: &Path, rel: &str, out: &mut Vec<String>) -> io::Result<()> {
    let full = root.join(rel);
    let Ok(meta) = fs::symlink_metadata(&full) else { return Ok(()) };
    if meta.is_dir() {
        for e in fs::read_dir(&full)? {
            let name = e?.file_name().to_string_lossy().into_owned();
            if name != ".svn" && name != ".git" {
                walk(root, &format!("{rel}/{name}"), out)?;
            }
        }
    } else if meta.is_file() {
        out.push(rel.to_string());
    }
    Ok(())
}

static SEQ: AtomicU64 = AtomicU64::new(0);

/// paths 를 복사해 둔다. 남길 파일이 없으면 None. 실패하면 Err — 부르는 쪽은 쓰기를 멈춘다.
pub fn save(base: &Path, what: &str, root: &Path, paths: &[String]) -> Result<Option<Saved>, String> {
    let fail = |e: io::Error| format!("복사본을 남기지 못해 멈췄습니다 ({e}). 파일은 그대로입니다. 디스크 공간과 {} 을 보세요.", slash(base));
    let mut all = Vec::new();
    for p in paths.iter().filter(|p| safe_rel(p)) {
        walk(root, p.trim_end_matches('/'), &mut all).map_err(fail)?;
    }
    if all.is_empty() {
        return Ok(None);
    }
    prune(base);
    let when_ms = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0);
    let id = format!("{when_ms}-{}-{}", std::process::id(), SEQ.fetch_add(1, Ordering::Relaxed));
    let dir = base.join(&id);
    let mut saved = Saved { id, what: what.to_string(), root: slash(root), when_ms, files: Vec::new(), skipped: Vec::new() };
    let copy = |saved: &mut Saved| -> io::Result<()> {
        for rel in &all {
            if fs::metadata(root.join(rel))?.len() > FILE_MAX {
                saved.skipped.push(rel.clone());
                continue;
            }
            let to = dir.join("files").join(rel);
            if let Some(parent) = to.parent() {
                fs::create_dir_all(parent)?;
            }
            fs::copy(root.join(rel), &to)?;
            saved.files.push(rel.clone());
        }
        fs::create_dir_all(&dir)?;
        fs::write(dir.join("manifest.json"), serde_json::to_vec(&*saved)?)
    };
    if let Err(e) = copy(&mut saved) {
        let _ = fs::remove_dir_all(&dir);
        return Err(fail(e));
    }
    Ok(Some(saved))
}

fn prune(base: &Path) {
    let Ok(rd) = fs::read_dir(base) else { return };
    for e in rd.filter_map(Result::ok) {
        let old = e.metadata().and_then(|m| m.modified()).ok().and_then(|t| t.elapsed().ok()).is_some_and(|age| age > KEEP);
        if old {
            let _ = fs::remove_dir_all(e.path());
        }
    }
}

/// 이 작업 폴더의 복사본 — 최신이 앞
pub fn list(base: &Path, root: &Path) -> Vec<Saved> {
    let Ok(rd) = fs::read_dir(base) else { return Vec::new() };
    let root = slash(root);
    let mut out: Vec<Saved> = rd
        .filter_map(Result::ok)
        .filter_map(|e| fs::read(e.path().join("manifest.json")).ok())
        .filter_map(|b| serde_json::from_slice::<Saved>(&b).ok())
        .filter(|s| s.root.eq_ignore_ascii_case(&root))
        .collect();
    out.sort_by(|a, b| b.when_ms.cmp(&a.when_ms));
    out
}

/// 복사본 하나에서 파일을 작업 폴더로 되돌린다. rel 이 그 복사본에 없으면 Err.
pub fn restore_file(base: &Path, id: &str, rel: &str, root: &Path) -> Result<(), String> {
    if id.contains(['/', '\\']) || id.contains("..") || !safe_rel(rel) {
        return Err(format!("복사본 경로가 올바르지 않습니다: {rel}"));
    }
    let from = base.join(id).join("files").join(rel);
    let to = root.join(rel);
    if let Some(parent) = to.parent() {
        fs::create_dir_all(parent).map_err(|e| format!("폴더를 만들지 못했습니다: {e}"))?;
    }
    fs::copy(&from, &to).map(|_| ()).map_err(|e| format!("{rel} 을 되살리지 못했습니다: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 남기고_목록을_보고_되살린다() {
        let t = std::env::temp_dir().join(format!("tomcatrun-backup-{}", std::process::id()));
        let _ = fs::remove_dir_all(&t);
        let (base, root) = (t.join("base"), t.join("wc"));
        fs::create_dir_all(root.join("a/.svn")).unwrap();
        fs::write(root.join("a/x.txt"), "내 변경").unwrap();
        fs::write(root.join("a/.svn/wc.db"), "db").unwrap();
        // 없는 경로 · 맨 위 · 밖으로 나가는 경로는 남기지 않는다
        assert!(save(&base, "Revert", &root, &["없음".into(), ".".into(), "../x".into()]).unwrap().is_none());
        let s = save(&base, "Revert", &root, &["a".into()]).unwrap().unwrap();
        assert_eq!(s.files, ["a/x.txt"]);
        fs::write(root.join("a/x.txt"), "덮임").unwrap();
        assert_eq!(list(&base, &root)[0].id, s.id);
        assert!(list(&base, &t).is_empty());
        restore_file(&base, &s.id, "a/x.txt", &root).unwrap();
        assert_eq!(fs::read_to_string(root.join("a/x.txt")).unwrap(), "내 변경");
        assert!(restore_file(&base, "../up", "a/x.txt", &root).is_err());
        assert!(restore_file(&base, &s.id, "없음", &root).is_err());
        let _ = fs::remove_dir_all(&t);
    }
}
