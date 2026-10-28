// 읽기 전용 SVN — 작업복사본의 .svn/wc.db 를 읽기 전용 · immutable 로 열어 상태와 차이를 계산한다.
//
// svn 명령을 쓰지 않고, 작업복사본(.svn 포함)에 아무것도 쓰지 않는다.
// 커밋 · 업데이트 · 되돌리기 같은 쓰기는 하지 않는다 — 전역 규칙(SVN 반영 명령 실행 금지)이다. 화면은 명령을 복사해 줄 뿐이다.
//
// 판정은 svn status 와 같은 순서다.
//   기록된 크기 · 수정 시각(translated_size · last_mod_time)이 같으면 그대로, 다르면 내용 SHA1 을 기준본 체크섬과 견준다.
//   이 저장소의 파일에는 svn:eol-style · svn:keywords 가 없어(2026-10-27 확인) 기준본과 바이트 그대로 견주면 된다.
//   기준본은 .svn/pristine/<앞 두 글자>/<sha1>.svn-base 에 압축 없이 있다 (형식 31).

use std::collections::{HashMap, HashSet};
use std::fs;
use std::io::Read;
use std::path::Path;
use std::time::{Instant, UNIX_EPOCH};

use rusqlite::{Connection, OpenFlags};
use serde::Serialize;

// svn 기본 global-ignores (svn 1.8 의 기본값)
const DEFAULT_IGNORES: [&str; 16] = [
    "*.o", "*.lo", "*.la", "*.al", ".libs", "*.so", "*.so.[0-9]*", "*.a", "*.pyc", "*.pyo", "__pycache__", "*.rej", "*~", "#*#",
    ".#*", ".*.swp",
];

// 차이를 보여 줄 파일 크기 상한. 넘으면 「너무 커서 차이를 보이지 않습니다」.
const DIFF_MAX_BYTES: u64 = 3 * 1024 * 1024;

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Item {
    pub path: String,
    pub code: &'static str, // M 수정 · A 추가 · D 삭제 · ! 없어짐 · C 충돌 · P 속성만 · ~ 종류 바뀜 · ? 추가 안 됨
    pub label: &'static str,
    pub dir: bool,
    pub conflict: bool,
    pub prop_changed: bool,
    pub rev: Option<i64>,         // 기준본의 마지막 커밋 리비전
    pub author: Option<String>,   // 그 커밋의 작성자
    pub date_ms: Option<i64>,     // 그 커밋 시각
    #[serde(skip_serializing_if = "Option::is_none")]
    pub orig: Option<String>,     // 이름을 바꾼 파일의 옛 경로 (git 만)
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Status {
    pub url: String,
    pub rev_min: i64,
    pub rev_max: i64,
    pub nodes: usize,
    pub hashed: usize,
    pub took_ms: u64,
    pub items: Vec<Item>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DiffLine {
    pub t: &'static str, // ctx · add · del · hunk
    pub o: Option<usize>,
    pub n: Option<usize>,
    pub s: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Diff {
    pub path: String,
    pub note: Option<String>, // 바이너리 · 너무 큼 · 폴더 같은 까닭으로 줄 차이를 못 보일 때
    pub added: usize,
    pub removed: usize,
    pub lines: Vec<DiffLine>,
}

struct Node {
    op: i64,
    presence: String,
    kind: String,
    checksum: Option<String>,
    size: Option<i64>,
    mtime: Option<i64>,
    crev: Option<i64>,
    cauthor: Option<String>,
    cdate: Option<i64>,
    props: Option<Vec<u8>>,
}

fn open(wc: &Path) -> Result<Connection, String> {
    let db = wc.join(".svn").join("wc.db");
    if !db.exists() {
        return Err(format!("SVN 작업복사본이 아닙니다 (.svn/wc.db 없음): {}", db.display()));
    }
    // immutable=1 : 잠금도 저널도 만들지 않고 읽기만 한다. IDE · svn 이 쓰는 중이면 읽다 어긋날 수 있어 부르는 쪽이 다시 시도한다.
    let uri = format!("file:{}?immutable=1", db.to_string_lossy().replace('\\', "/"));
    Connection::open_with_flags(uri, OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_URI | OpenFlags::SQLITE_OPEN_NO_MUTEX)
        .map_err(|e| format!("wc.db 를 열지 못했습니다: {e}"))
}

fn load_nodes(c: &Connection) -> rusqlite::Result<HashMap<String, Node>> {
    // 경로마다 가장 위 층(op_depth 최대)이 지금 상태다 — 0 은 기준(BASE), 1 이상은 추가 · 삭제 · 복사 같은 작업 중 변경
    let mut st = c.prepare(
        "SELECT local_relpath, op_depth, presence, kind, checksum, translated_size, last_mod_time,
                changed_revision, changed_author, changed_date, properties
         FROM NODES n
         WHERE op_depth = (SELECT MAX(op_depth) FROM NODES m WHERE m.wc_id = n.wc_id AND m.local_relpath = n.local_relpath)",
    )?;
    let rows = st.query_map([], |r| {
        Ok((
            r.get::<_, String>(0)?,
            Node {
                op: r.get(1)?,
                presence: r.get(2)?,
                kind: r.get(3)?,
                checksum: r.get(4)?,
                size: r.get(5)?,
                mtime: r.get(6)?,
                crev: r.get(7)?,
                cauthor: r.get(8)?,
                cdate: r.get(9)?,
                props: r.get(10)?,
            },
        ))
    })?;
    rows.collect()
}

/// svn skel — 「( … )」 목록, 「길이 공백 바이트」 원자, 맨 단어 원자. 속성 목록을 읽는 데만 쓴다.
#[derive(Debug)]
enum Skel {
    Atom(Vec<u8>),
    List(Vec<Skel>),
}

fn parse_skel(b: &[u8], mut i: usize) -> Option<(Skel, usize)> {
    while i < b.len() && b[i].is_ascii_whitespace() {
        i += 1;
    }
    if i >= b.len() {
        return None;
    }
    if b[i] == b'(' {
        i += 1;
        let mut out = Vec::new();
        loop {
            while i < b.len() && b[i].is_ascii_whitespace() {
                i += 1;
            }
            if i >= b.len() {
                return None;
            }
            if b[i] == b')' {
                return Some((Skel::List(out), i + 1));
            }
            let (item, j) = parse_skel(b, i)?;
            out.push(item);
            i = j;
        }
    }
    if b[i].is_ascii_digit() {
        let mut j = i;
        while j < b.len() && b[j].is_ascii_digit() {
            j += 1;
        }
        let n: usize = std::str::from_utf8(&b[i..j]).ok()?.parse().ok()?;
        let start = j + 1;
        let end = start.checked_add(n)?;
        if end > b.len() {
            return None;
        }
        return Some((Skel::Atom(b[start..end].to_vec()), end));
    }
    let mut j = i;
    while j < b.len() && !b[j].is_ascii_whitespace() && b[j] != b'(' && b[j] != b')' {
        j += 1;
    }
    Some((Skel::Atom(b[i..j].to_vec()), j))
}

fn prop(blob: &Option<Vec<u8>>, name: &str) -> Option<String> {
    let (Skel::List(items), _) = parse_skel(blob.as_ref()?, 0)? else { return None };
    items.chunks(2).find_map(|kv| match kv {
        [Skel::Atom(k), Skel::Atom(v)] if k.as_slice() == name.as_bytes() => Some(String::from_utf8_lossy(v).into_owned()),
        _ => None,
    })
}

/// svn 무시 패턴 (fnmatch) — * ? [..] 만
fn glob(p: &[u8], s: &[u8]) -> bool {
    match (p.first(), s.first()) {
        (None, None) => true,
        (Some(b'*'), _) => glob(&p[1..], s) || (!s.is_empty() && glob(p, &s[1..])),
        (Some(b'?'), Some(_)) => glob(&p[1..], &s[1..]),
        (Some(b'['), Some(&c)) => {
            let Some(close) = p.iter().skip(1).position(|&x| x == b']').map(|k| k + 1) else { return false };
            let set = &p[1..close];
            let mut hit = false;
            let mut k = 0;
            while k < set.len() {
                if k + 2 < set.len() && set[k + 1] == b'-' {
                    hit |= set[k] <= c && c <= set[k + 2];
                    k += 3;
                } else {
                    hit |= set[k] == c;
                    k += 1;
                }
            }
            hit && glob(&p[close + 1..], &s[1..])
        }
        (Some(&a), Some(&b)) => a == b && glob(&p[1..], &s[1..]),
        _ => false,
    }
}

fn sha1_file(path: &Path) -> Option<String> {
    let mut f = fs::File::open(path).ok()?;
    let mut h = sha1_smol::Sha1::new();
    let mut buf = vec![0u8; 1 << 20];
    loop {
        let n = f.read(&mut buf).ok()?;
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
    }
    Some(h.digest().to_string())
}

fn mtime_us(meta: &fs::Metadata) -> Option<i64> {
    meta.modified().ok()?.duration_since(UNIX_EPOCH).ok().map(|d| d.as_micros() as i64)
}

fn label(code: &str) -> &'static str {
    match code {
        "M" => "수정",
        "A" => "추가",
        "D" => "삭제",
        "!" => "없어짐",
        "C" => "충돌",
        "P" => "속성 변경",
        "~" => "종류 바뀜",
        _ => "추가 안 됨",
    }
}

pub fn status(wc: &Path) -> Result<Status, String> {
    let began = Instant::now();
    let c = open(wc)?;
    // 쓰는 중에 읽혀 어긋나면 한 번 더 읽는다
    let nodes = load_nodes(&c).or_else(|_| load_nodes(&c)).map_err(|e| format!("wc.db 를 읽지 못했습니다: {e}"))?;
    let conflicts: HashSet<String> = c
        .prepare("SELECT local_relpath FROM ACTUAL_NODE WHERE conflict_data IS NOT NULL")
        .and_then(|mut s| s.query_map([], |r| r.get::<_, String>(0))?.collect())
        .unwrap_or_default();
    let propmods: HashSet<String> = c
        .prepare("SELECT local_relpath FROM ACTUAL_NODE WHERE properties IS NOT NULL")
        .and_then(|mut s| s.query_map([], |r| r.get::<_, String>(0))?.collect())
        .unwrap_or_default();
    let (url, rev_min, rev_max) = c
        .query_row(
            "SELECT r.root || '/' || n.repos_path,
                    (SELECT MIN(revision) FROM NODES WHERE op_depth = 0 AND presence = 'normal'),
                    (SELECT MAX(revision) FROM NODES WHERE op_depth = 0 AND presence = 'normal')
             FROM NODES n JOIN REPOSITORY r ON r.id = n.repos_id WHERE n.local_relpath = '' AND n.op_depth = 0",
            [],
            |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?, r.get::<_, i64>(2)?)),
        )
        .unwrap_or_default();
    drop(c);

    let mut items: HashMap<String, Item> = HashMap::new();
    let mut missing_dirs: Vec<String> = Vec::new();
    let mut hashed = 0;
    let mk = |path: &str, code: &'static str, dir: bool, n: Option<&Node>| Item {
        path: path.to_string(),
        code,
        label: label(code),
        dir,
        conflict: false,
        prop_changed: false,
        rev: n.and_then(|n| n.crev),
        author: n.and_then(|n| n.cauthor.clone()),
        date_ms: n.and_then(|n| n.cdate.map(|d| d / 1000)),
        orig: None,
    };
    let mut rels: Vec<&String> = nodes.keys().collect();
    rels.sort(); // 부모가 먼저 오게 — 없어진 폴더 아래는 하나하나 늘어놓지 않는다
    for rel in rels {
        let n = &nodes[rel];
        if rel.is_empty() || missing_dirs.iter().any(|d| rel.starts_with(&format!("{d}/"))) {
            continue;
        }
        let dir = n.kind == "dir";
        if n.presence == "base-deleted" {
            items.insert(rel.clone(), mk(rel, "D", dir, Some(n)));
            continue;
        }
        if n.presence != "normal" {
            continue; // not-present · excluded · server-excluded
        }
        let full = wc.join(rel);
        let meta = fs::symlink_metadata(&full);
        let Ok(meta) = meta else {
            items.insert(rel.clone(), mk(rel, "!", dir, Some(n)));
            if dir {
                missing_dirs.push(rel.clone());
            }
            continue;
        };
        if meta.is_dir() != dir {
            items.insert(rel.clone(), mk(rel, "~", dir, Some(n)));
            continue;
        }
        if n.op > 0 {
            items.insert(rel.clone(), mk(rel, "A", dir, None));
            continue;
        }
        if dir {
            continue;
        }
        // svn 과 같은 빠른 길 — 크기와 수정 시각이 기록과 같으면 열어 보지 않는다
        if n.size == Some(meta.len() as i64) && n.mtime == mtime_us(&meta) {
            continue;
        }
        hashed += 1;
        let same = match (&n.checksum, sha1_file(&full)) {
            (Some(ck), Some(h)) => ck.strip_prefix("$sha1$") == Some(h.as_str()),
            _ => false,
        };
        if !same {
            items.insert(rel.clone(), mk(rel, "M", false, Some(n)));
        }
    }
    for rel in conflicts {
        let n = nodes.get(&rel);
        let e = items.entry(rel.clone()).or_insert_with(|| mk(&rel, "C", n.map_or(false, |n| n.kind == "dir"), n));
        e.conflict = true;
    }
    for rel in propmods {
        let n = nodes.get(&rel);
        let shown = if rel.is_empty() { ".".to_string() } else { rel.clone() };
        let e = items.entry(shown.clone()).or_insert_with(|| mk(&shown, "P", n.map_or(true, |n| n.kind == "dir"), n));
        e.prop_changed = true;
    }

    // 추가 안 된 새 파일 — 버전 관리되는 폴더마다 디스크의 이름을 견준다. 무시 목록(svn:ignore · 기본값)에 걸리면 뺀다.
    for (rel, n) in &nodes {
        if n.kind != "dir" || n.presence != "normal" || missing_dirs.contains(rel) {
            continue;
        }
        let ignores: Vec<String> = prop(&n.props, "svn:ignore").map(|v| v.split_whitespace().map(String::from).collect()).unwrap_or_default();
        let Ok(rd) = fs::read_dir(wc.join(rel)) else { continue };
        for e in rd.filter_map(Result::ok) {
            let name = e.file_name().to_string_lossy().into_owned();
            if name == ".svn" {
                continue;
            }
            let child = if rel.is_empty() { name.clone() } else { format!("{rel}/{name}") };
            if nodes.contains_key(&child) {
                continue;
            }
            let ignored = ignores.iter().map(String::as_str).chain(DEFAULT_IGNORES).any(|p| glob(p.as_bytes(), name.as_bytes()));
            if ignored {
                continue;
            }
            let is_dir = e.file_type().map(|t| t.is_dir()).unwrap_or(false);
            items.insert(child.clone(), mk(&child, "?", is_dir, None));
        }
    }

    const ORDER: [&str; 8] = ["C", "M", "A", "D", "!", "~", "P", "?"];
    let mut list: Vec<Item> = items.into_values().collect();
    list.sort_by(|a, b| {
        let ka = ORDER.iter().position(|c| *c == a.code).unwrap_or(9);
        let kb = ORDER.iter().position(|c| *c == b.code).unwrap_or(9);
        ka.cmp(&kb).then_with(|| a.path.cmp(&b.path))
    });
    Ok(Status { url, rev_min, rev_max, nodes: nodes.len(), hashed, took_ms: began.elapsed().as_millis() as u64, items: list })
}

// UTF-8 이 아니면 MS949 로 읽는다 (로그와 같은 규칙)
fn decode(b: &[u8]) -> String {
    match std::str::from_utf8(b) {
        Ok(s) => s.to_string(),
        Err(_) => encoding_rs::EUC_KR.decode(b).0.into_owned(),
    }
}

fn pristine(wc: &Path, checksum: &str) -> Option<Vec<u8>> {
    let sha = checksum.strip_prefix("$sha1$")?;
    fs::read(wc.join(".svn").join("pristine").join(&sha[..2]).join(format!("{sha}.svn-base"))).ok()
}

/// 기준본(마지막으로 받은 내용)과 지금 파일의 차이. 추가 · 새 파일은 빈 것과, 삭제 · 없어짐은 기준본과 빈 것을 견준다.
pub fn diff(wc: &Path, rel: &str) -> Result<Diff, String> {
    let c = open(wc)?;
    let base: Option<(String, Option<String>)> = c
        .query_row(
            "SELECT kind, checksum FROM NODES WHERE local_relpath = ?1 AND op_depth = 0 AND presence = 'normal'",
            [rel],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .ok();
    drop(c);
    let full = wc.join(rel);
    let mut out = Diff { path: rel.to_string(), note: None, added: 0, removed: 0, lines: Vec::new() };
    if full.is_dir() || base.as_ref().map_or(false, |(k, _)| k == "dir") {
        out.note = Some("폴더입니다. 안의 파일을 고르면 차이를 보여 줍니다.".into());
        return Ok(out);
    }
    let old = match &base {
        Some((_, Some(ck))) => pristine(wc, ck).ok_or("기준본 파일을 찾지 못했습니다 (.svn/pristine).")?,
        _ => Vec::new(),
    };
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
        out.note = Some("내용이 기준본과 같습니다 (수정 시각만 바뀜).".into());
    }
    Ok(out)
}

/// 두 내용의 줄 차이 — 작업복사본 · 기준본 · 저장소 리비전 어느 쪽끼리든 같은 꼴로 보여 준다
pub fn text_diff(rel: &str, old: &[u8], new: &[u8]) -> Diff {
    let mut out = Diff { path: rel.to_string(), note: None, added: 0, removed: 0, lines: Vec::new() };
    if old.len() as u64 > DIFF_MAX_BYTES || new.len() as u64 > DIFF_MAX_BYTES {
        out.note = Some("파일이 커서(3MB 넘음) 차이를 보이지 않습니다.".into());
        return out;
    }
    let binary = |b: &[u8]| b.iter().take(8192).any(|&x| x == 0);
    if binary(old) || binary(new) {
        out.note = Some(if old == new { "바이너리 파일 · 내용 같음".into() } else { "바이너리 파일이라 줄 차이를 보이지 않습니다 (내용은 다름).".into() });
        return out;
    }
    let (a, b) = (decode(old), decode(new));
    let td = similar::TextDiff::from_lines(&a, &b);
    for group in td.grouped_ops(3) {
        let (first, last) = (group.first().unwrap(), group.last().unwrap());
        let (o0, o1) = (first.old_range().start, last.old_range().end);
        let (n0, n1) = (first.new_range().start, last.new_range().end);
        out.lines.push(DiffLine { t: "hunk", o: None, n: None, s: format!("@@ -{},{} +{},{} @@", o0 + 1, o1 - o0, n0 + 1, n1 - n0) });
        for op in &group {
            for ch in td.iter_changes(op) {
                let t = match ch.tag() {
                    similar::ChangeTag::Equal => "ctx",
                    similar::ChangeTag::Insert => {
                        out.added += 1;
                        "add"
                    }
                    similar::ChangeTag::Delete => {
                        out.removed += 1;
                        "del"
                    }
                };
                let s = ch.value().trim_end_matches(['\r', '\n']).to_string();
                out.lines.push(DiffLine { t, o: ch.old_index().map(|x| x + 1), n: ch.new_index().map(|x| x + 1), s });
            }
        }
    }
    if out.lines.is_empty() && old == new {
        out.note = Some("두 내용이 같습니다.".into());
    }
    out
}

/// 작업복사본의 저장소 위치
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WcInfo {
    pub repo_root: String, // 예: http://svn.example.com/svn/repo
    pub repos_path: String, // 예: trunk/shop
    pub url: String,
    pub rev_min: i64,
    pub rev_max: i64,
}

pub fn wc_info(wc: &Path) -> Result<WcInfo, String> {
    let c = open(wc)?;
    c.query_row(
        "SELECT r.root, n.repos_path,
                (SELECT MIN(revision) FROM NODES WHERE op_depth = 0 AND presence = 'normal'),
                (SELECT MAX(revision) FROM NODES WHERE op_depth = 0 AND presence = 'normal')
         FROM NODES n JOIN REPOSITORY r ON r.id = n.repos_id WHERE n.local_relpath = '' AND n.op_depth = 0",
        [],
        |r| {
            let root: String = r.get(0)?;
            let rp: String = r.get(1)?;
            Ok(WcInfo { url: format!("{}/{}", root.trim_end_matches('/'), rp), repo_root: root, repos_path: rp, rev_min: r.get(2)?, rev_max: r.get(3)? })
        },
    )
    .map_err(|e| format!("작업복사본 정보를 읽지 못했습니다: {e}"))
}

/// 기준본 내용 (마지막으로 받은 것) — 버전 관리 밖이면 None
pub fn base_content(wc: &Path, rel: &str) -> Option<Vec<u8>> {
    let c = open(wc).ok()?;
    let ck: Option<String> = c
        .query_row("SELECT checksum FROM NODES WHERE local_relpath = ?1 AND op_depth = 0 AND presence = 'normal' AND kind = 'file'", [rel], |r| r.get(0))
        .ok()?;
    pristine(wc, &ck?)
}

/// 작업복사본 변경을 알아채기 위한 표지 — IDE · svn 이 커밋 · 업데이트하면 wc.db 가 바뀐다
pub fn db_stamp(wc: &Path) -> Option<u64> {
    let m = fs::metadata(wc.join(".svn").join("wc.db")).ok()?;
    m.modified().ok()?.duration_since(UNIX_EPOCH).ok().map(|d| d.as_millis() as u64)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 무시_패턴() {
        assert!(glob(b"*.class", b"A.class"));
        assert!(glob(b".*.swp", b".a.java.swp"));
        assert!(glob(b"*.so.[0-9]*", b"lib.so.1.2"));
        assert!(glob(b"[Tt]humbs.db", b"thumbs.db"));
        assert!(!glob(b"*.o", b"a.java"));
        assert!(glob(b"target", b"target"));
    }

    #[test]
    fn 속성_목록을_읽는다() {
        let blob = Some(b"(svn:ignore 18 .classpath\ntarget\n svn:mergeinfo 0 )".to_vec());
        assert_eq!(prop(&blob, "svn:ignore").as_deref(), Some(".classpath\ntarget\n"));
        assert_eq!(prop(&blob, "svn:eol-style"), None);
        assert_eq!(prop(&Some(b"()".to_vec()), "svn:ignore"), None);
    }

    // 실제 작업복사본을 읽기만 한다 — 환경 변수로 폴더를 줄 때만 (여럿이면 ; 로): TOMCATRUN_TEST_SVN_WC=<폴더> cargo test
    #[test]
    fn 실제_작업복사본_상태() {
        let dirs = std::env::var("TOMCATRUN_TEST_SVN_WC").unwrap_or_default();
        for wc in dirs.split(';').filter(|x| !x.trim().is_empty()) {
            let p = Path::new(wc);
            if !p.join(".svn/wc.db").exists() {
                continue;
            }
            let st = status(p).expect("상태를 계산한다");
            assert!(st.nodes > 1000 && st.url.starts_with("http"));
            let mut by: Vec<String> = Vec::new();
            for code in ["C", "M", "A", "D", "!", "~", "P", "?"] {
                let v: Vec<&str> = st.items.iter().filter(|i| i.code == code).map(|i| i.path.as_str()).collect();
                if !v.is_empty() {
                    by.push(format!("{code} {} {:?}", v.len(), &v[..v.len().min(4)]));
                }
            }
            println!("{wc} · {} · r{}~{} · 노드 {} · 해시 {} · {}ms\n  {}", st.url, st.rev_min, st.rev_max, st.nodes, st.hashed, st.took_ms, by.join("\n  "));
            // 바뀐 파일이 있으면 그 차이도 계산된다
            if let Some(m) = st.items.iter().find(|i| i.code == "M") {
                let d = diff(p, &m.path).expect("차이");
                assert!(d.note.is_some() || d.added + d.removed > 0, "{}", m.path);
            }
        }
    }
}
