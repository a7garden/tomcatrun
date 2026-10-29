// 기동 탐색 점검 — 「클래스 전체 탐색 건너뛰기」를 켜도 되는지, 「TLD jar」 칸에 무엇을 적으면 되는지 가린다.
//
// 2026-10-29 실측 : 둘 다 꺼 둔 설치본은 「웹앱 준비」가 17~36초였고, 같은 앱을 둘 다 켜고 띄운 개발 폴더는 1.6~1.8초였다.
// jar 230개 중 TLD 가 든 것은 7개, web-fragment · 초기화 클래스가 든 것은 spring-web 하나뿐이었다.
// 그런데 두 칸은 사람이 정해야 해서 대부분 꺼진 채로 쓰였다 — jar 와 소스를 직접 읽어 켜도 되는지 알려 준다.
//
// 저절로 켜지는 않는다. 이름을 바꾼 jar · 리플렉션 등록처럼 여기서 못 보는 길이 있어 알리기만 한다.

use std::fs::{self, File};
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

#[derive(Clone, Default)]
pub struct JarReport {
    pub total: usize,            // 본 jar 수
    pub tld: Vec<String>,       // META-INF/**/*.tld 가 든 jar
    pub fragment: Vec<String>,   // META-INF/web-fragment.xml 이 든 jar
    pub sci: Vec<String>,        // META-INF/services/javax.servlet.ServletContainerInitializer 가 든 jar
    pub unreadable: Vec<String>, // 목록을 읽지 못한 jar (ZIP64 · 깨진 파일)
}

// spring-web 의 web-fragment · 초기화 클래스(SpringServletContainerInitializer)는 소스에 WebApplicationInitializer 구현이 없으면 아무것도 하지 않는다.
// 그래서 이것만 있으면 건너뛰어도 된다. 소스 쪽은 src_hits 가 본다.
const HARMLESS: [&str; 1] = ["spring-web"];

/// 폴더들의 jar 를 열어 목록(중앙 디렉터리)만 읽는다. 압축은 풀지 않는다.
pub fn scan_jars(dirs: &[&Path]) -> JarReport {
    let mut r = JarReport::default();
    for dir in dirs {
        let Ok(rd) = fs::read_dir(dir) else { continue };
        let mut names: Vec<_> = rd.filter_map(Result::ok).map(|e| e.path()).filter(|p| p.extension().map_or(false, |x| x.eq_ignore_ascii_case("jar"))).collect();
        names.sort();
        for path in names {
            let name = path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
            r.total += 1;
            let Some(entries) = zip_names(&path) else {
                r.unreadable.push(name);
                continue;
            };
            let has = |f: &dyn Fn(&str) -> bool| entries.iter().any(|e| f(e));
            if has(&|e| e.starts_with("META-INF/") && e.ends_with(".tld")) {
                r.tld.push(name.clone());
            }
            if has(&|e| e == "META-INF/web-fragment.xml") {
                r.fragment.push(name.clone());
            }
            if has(&|e| e == "META-INF/services/javax.servlet.ServletContainerInitializer") {
                r.sci.push(name.clone());
            }
        }
    }
    r
}

/// jar(zip) 안의 파일 이름들. 끝의 중앙 디렉터리만 읽는다 — 230개 jar 를 통째로 읽지 않으려는 것이다.
fn zip_names(path: &Path) -> Option<Vec<String>> {
    let mut f = File::open(path).ok()?;
    let len = f.metadata().ok()?.len();
    // 끝 레코드(EOCD)는 22바이트 + 주석 최대 65535바이트 안에 있다
    let tail = len.min(22 + 65535);
    f.seek(SeekFrom::Start(len - tail)).ok()?;
    let mut buf = vec![0u8; tail as usize];
    f.read_exact(&mut buf).ok()?;
    let at = (0..buf.len().saturating_sub(21)).rev().find(|&k| buf[k..k + 4] == [0x50, 0x4b, 0x05, 0x06])?;
    let u16le = |b: &[u8], o: usize| u16::from_le_bytes([b[o], b[o + 1]]) as usize;
    let u32le = |b: &[u8], o: usize| u32::from_le_bytes([b[o], b[o + 1], b[o + 2], b[o + 3]]) as u64;
    let (size, offset) = (u32le(&buf, at + 12), u32le(&buf, at + 16));
    // ZIP64 는 값이 0xFFFFFFFF 로 막혀 있다 — 읽지 못한 것으로 둔다
    if offset == 0xFFFF_FFFF || size == 0xFFFF_FFFF || offset + size > len {
        return None;
    }
    let mut cd = vec![0u8; size as usize];
    f.seek(SeekFrom::Start(offset)).ok()?;
    f.read_exact(&mut cd).ok()?;
    let mut out = Vec::new();
    let mut k = 0;
    while k + 46 <= cd.len() && cd[k..k + 4] == [0x50, 0x4b, 0x01, 0x02] {
        let (n, e, c) = (u16le(&cd, k + 28), u16le(&cd, k + 30), u16le(&cd, k + 32));
        let end = (k + 46 + n).min(cd.len());
        out.push(String::from_utf8_lossy(&cd[k + 46..end]).to_string());
        k += 46 + n + e + c;
    }
    Some(out)
}

// 소스에 이것이 보이면 톰캣이 클래스를 훑어야 찾는 등록이 있다 — 건너뛰면 그 서블릿 · 필터 · 초기화가 조용히 빠진다
const SRC_MARKS: [&str; 6] = ["@WebServlet", "@WebFilter", "@WebListener", "@HandlesTypes", "WebApplicationInitializer", "ServletContainerInitializer"];

/// 소스 폴더에서 위 표지가 든 파일 (프로젝트 기준 경로). 주석 속 낱말도 센다 — 놓치는 것보다 낫다.
pub fn src_hits(src: &Path) -> Vec<String> {
    fn walk(dir: &Path, root: &Path, out: &mut Vec<String>) {
        let Ok(rd) = fs::read_dir(dir) else { return };
        for e in rd.filter_map(Result::ok) {
            let path = e.path();
            if path.is_dir() {
                walk(&path, root, out);
            } else if path.extension().map_or(false, |x| x == "java") {
                let Ok(b) = fs::read(&path) else { continue };
                if SRC_MARKS.iter().any(|m| b.windows(m.len()).any(|w| w == m.as_bytes())) {
                    out.push(path.strip_prefix(root).unwrap_or(&path).to_string_lossy().replace('\\', "/"));
                }
            }
        }
    }
    let mut out = Vec::new();
    walk(src, src, &mut out);
    out.sort();
    out
}

fn lib_key(jar: &str) -> String {
    crate::jarcheck::split_version(jar).0
}

/// 「TLD jar」 칸에 적을 꼴 — 「jstl-1.2.jar」 → 「jstl-*.jar」. 판이 없는 이름은 그대로.
pub fn tld_pattern(jar: &str) -> String {
    let (key, ver) = crate::jarcheck::split_version(jar);
    if ver.is_empty() { jar.to_ascii_lowercase() } else { format!("{key}-*.jar") }
}

/// 건너뛰면 빠지는 것 — 비면 건너뛰어도 된다
pub fn skip_blockers(jars: &JarReport, src: &[String]) -> Vec<String> {
    let mut v = Vec::new();
    let other = |list: &[String]| list.iter().filter(|j| !HARMLESS.contains(&lib_key(j).as_str())).cloned().collect::<Vec<_>>();
    let frag = other(&jars.fragment);
    if !frag.is_empty() {
        v.push(format!("web-fragment.xml 이 든 jar: {}", frag.join(", ")));
    }
    let sci = other(&jars.sci);
    if !sci.is_empty() {
        v.push(format!("초기화 클래스(ServletContainerInitializer)가 든 jar: {}", sci.join(", ")));
    }
    if !src.is_empty() {
        let more = if src.len() > 3 { format!(" 외 {}개", src.len() - 3) } else { String::new() };
        v.push(format!("애너테이션 · 초기화 클래스를 쓰는 소스: {}{more}", src.iter().take(3).cloned().collect::<Vec<_>>().join(", ")));
    }
    if !jars.unreadable.is_empty() {
        v.push(format!("목록을 읽지 못한 jar: {}", jars.unreadable.join(", ")));
    }
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tld_pattern_drops_version() {
        assert_eq!(tld_pattern("jstl-1.2.jar"), "jstl-*.jar");
        assert_eq!(tld_pattern("spring-webmvc-4.3.22.RELEASE.jar"), "spring-webmvc-*.jar");
        assert_eq!(tld_pattern("egovframework.rte.ptl.mvc-3.9.0.jar"), "egovframework.rte.ptl.mvc-*.jar");
        assert_eq!(tld_pattern("Foo.jar"), "foo.jar");
    }

    #[test]
    fn spring_web_alone_does_not_block() {
        let j = JarReport { fragment: vec!["spring-web-4.3.22.RELEASE.jar".into()], sci: vec!["spring-web-4.3.22.RELEASE.jar".into()], ..Default::default() };
        assert!(skip_blockers(&j, &[]).is_empty());
        let j = JarReport { sci: vec!["jersey-container-servlet-2.30.jar".into()], ..Default::default() };
        assert_eq!(skip_blockers(&j, &[]).len(), 1);
        assert_eq!(skip_blockers(&JarReport::default(), &["a/B.java".into()]).len(), 1);
    }

    #[test]
    fn reads_jar_names() {
        // 이 저장소가 받아 둔 jar 가 있으면 그것으로 — 없으면(CI) 건너뛴다
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../deps");
        let Some(jar) = fs::read_dir(&dir).ok().and_then(|rd| rd.filter_map(Result::ok).map(|e| e.path()).find(|p| p.extension().map_or(false, |x| x == "jar"))) else { return };
        let names = zip_names(&jar).expect("jar 목록");
        assert!(names.iter().any(|n| n.starts_with("META-INF/")), "{}", jar.display());
    }
}
