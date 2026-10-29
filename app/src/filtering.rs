// 리소스 필터링 점검 — pom.xml 의 <filtering>true</filtering> 로 Maven 이 값을 바꿔 넣을 자리표시자가 resources 에 있는지 센다.
//
// 실행기는 src/main/resources 원본을 그대로 /WEB-INF/classes 에 붙인다 (base/conf/server.xml 의 PreResources).
// 이클립스(m2e-wtp)는 Maven 처럼 걸러 낸 target/classes 를 배포하므로, Maven 이 값을 아는 ${key} · @key@ 는
// 이클립스에서는 값으로, 실행기에서는 글자 그대로 들어간다. 여기서는 걸러 넣지 않고 그런 자리를 세어 알리기만 한다.
// Spring 이 기동 때 properties 에서 채우는 ${db.url} 처럼 Maven 이 모르는 키는 Maven 도 그대로 두므로 세지 않는다.
//
// Maven(maven-resources-plugin 기본값)과 같게 하는 것
//   resource 의 directory · filtering · includes · excludes — 켠 프로필의 resources 는 뒤에 붙는다
//   값을 찾는 곳   <filters> 의 .properties → 프로젝트 <properties> → 켠 프로필의 <properties> (뒤가 앞을 덮는다)
//                  project.* · pom.* · basedir (모델 값) · env.* (환경 변수) · 자바 시스템 속성 몇 가지
//   구분자 ${…} · @…@ · 거르지 않는 확장자 jpg · jpeg · gif · bmp · png (+ pom 의 nonFilteredFileExtension)
// 켠 프로필은 이클립스(m2e)를 따른다 — <프로젝트>/.settings/org.eclipse.m2e.core.prefs 의 activeProfiles (「!x」 는 끄기).
// 거기 켠 것이 없으면 activeByDefault 프로필이다.
//
// 판단하지 않는 것 — 보이면 「판단 못 함」으로 알린다
//   조건(속성 · OS · JDK · 파일)으로 켜지는 프로필 · 상위 pom 에서 물려받은 속성 · 플러그인 설정의 delimiters · escapeString
//   (여러 resource 가 같은 파일을 덮어쓰는 순서와 targetPath 는 보지 않는다 — 걸러지는 파일이면 센다)
//
// 목차 : 읽기(Plan · plan · active_profiles · read_props) · 세기(scan · placeholders) · 알림(Report)

use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::Path;

use roxmltree::Node;

/// 이클립스(m2e)가 프로젝트마다 켠 Maven 프로필을 적는 곳
pub const M2E_PREFS: &str = ".settings/org.eclipse.m2e.core.prefs";
// 실행기가 /WEB-INF/classes 로 올리는 폴더 — 다른 폴더는 걸러져도 실행기와 상관없다
const RESOURCES: &str = "src/main/resources";
// maven-resources-plugin 이 기본으로 거르지 않는 확장자
const NON_FILTERED: [&str; 5] = ["jpg", "jpeg", "gif", "bmp", "png"];
// 이보다 크면 설정 파일이 아니라고 보고 읽지 않는다 (점검이 상태 화면을 붙잡지 않게)
const MAX_BYTES: u64 = 4 * 1024 * 1024;
// Maven 이 값으로 바꾸는 자바 시스템 속성 중 설정 파일에 흔히 쓰는 것
const SYSTEM_KEYS: [&str; 12] = [
    "user.home", "user.dir", "user.name", "java.home", "java.version", "java.io.tmpdir",
    "os.name", "os.arch", "file.separator", "path.separator", "line.separator", "file.encoding",
];
// 경고 한 줄에 보일 파일 수
const SHOWN: usize = 5;

/* ---------- 읽기 ---------- */

struct Resource {
    includes: Vec<String>, // 소문자 · 「/」 · resources 기준
    excludes: Vec<String>,
}

/// pom 과 m2e 설정에서 읽은 것
struct Plan {
    profiles: Vec<String>,
    filtered: Vec<Resource>,   // src/main/resources 를 거르는 resource
    vals: HashMap<String, String>,
    non_filtered: Vec<String>,
    notes: Vec<String>,        // 판단 못 함 · 실행기가 올리지 않는 폴더를 거름
}

impl Plan {
    /// Maven 이 이 키의 값을 아는지 — 알면 바꿔 넣고, 모르면 글자 그대로 둔다
    fn known(&self, key: &str) -> bool {
        self.vals.contains_key(key)
            || key == "basedir"
            || key.starts_with("project.")
            || key.starts_with("pom.")
            || key.strip_prefix("env.").map_or(false, |v| std::env::var_os(v).is_some())
            || SYSTEM_KEYS.contains(&key)
    }

    fn filters(&self, rel: &str) -> bool {
        let ext = rel.rsplit_once('.').map(|(_, e)| e).unwrap_or("");
        if self.non_filtered.iter().any(|x| x == ext) {
            return false;
        }
        self.filtered.iter().any(|r| (r.includes.is_empty() || r.includes.iter().any(|p| crate::glob(p, rel))) && !r.excludes.iter().any(|p| crate::glob(p, rel)))
    }
}

fn child<'a, 'i>(n: Node<'a, 'i>, name: &str) -> Option<Node<'a, 'i>> {
    n.children().find(|c| c.has_tag_name(name))
}

fn text(n: Option<Node>) -> String {
    n.and_then(|x| x.text()).map(str::trim).unwrap_or("").to_string()
}

fn list(n: Node, outer: &str, inner: &str) -> Vec<String> {
    child(n, outer).map(|o| o.children().filter(|c| c.has_tag_name(inner)).map(|c| text(Some(c))).filter(|t| !t.is_empty()).collect()).unwrap_or_default()
}

fn props_of(n: Node, into: &mut HashMap<String, String>) {
    if let Some(p) = child(n, "properties") {
        for c in p.children().filter(Node::is_element) {
            into.insert(c.tag_name().name().to_string(), text(Some(c)));
        }
    }
}

// pom 안 경로의 ${…} 를 아는 값으로 바꾼다. ${basedir} · ${project.basedir} 는 프로젝트 폴더라 앞에서 떼어 낸다.
fn interpolate(raw: &str, vals: &HashMap<String, String>) -> String {
    let mut s = raw.trim().replace('\\', "/");
    for pre in ["${project.basedir}/", "${basedir}/"] {
        if let Some(rest) = s.strip_prefix(pre) {
            s = rest.to_string();
        }
    }
    for (k, v) in vals {
        s = s.replace(&format!("${{{k}}}"), v);
    }
    s
}

fn norm_dir(raw: &str) -> String {
    let mut s = raw.trim().replace('\\', "/");
    while let Some(rest) = s.strip_prefix("./") {
        s = rest.to_string();
    }
    s.trim_end_matches('/').to_string()
}

// Ant 패턴 — 「dir/」 은 그 아래 전부
fn norm_pattern(raw: &str) -> String {
    let s = raw.trim().replace('\\', "/").to_ascii_lowercase();
    if s.ends_with('/') {
        format!("{s}**")
    } else {
        s
    }
}

/// m2e 의 activeProfiles 와 pom 의 activeByDefault 로 켠 프로필을 정한다.
/// 돌려주는 것 : 켠 프로필 · 판단 못 한 프로필 (조건으로 켜지고 m2e 에 적히지 않은 것)
fn active_profiles(m2e: &str, profiles: &[(String, bool, bool)]) -> (Vec<String>, Vec<String>) {
    let value = m2e.lines().find_map(|l| l.trim().strip_prefix("activeProfiles").and_then(|r| r.trim_start().strip_prefix('='))).unwrap_or("");
    let (mut on, mut off) = (Vec::new(), Vec::new());
    for t in value.split([',', ' ']).map(str::trim).filter(|t| !t.is_empty()) {
        match t.strip_prefix('!') {
            Some(x) => off.push(x.to_string()),
            None => on.push(t.to_string()),
        }
    }
    let ids: Vec<&String> = profiles.iter().map(|(id, _, _)| id).collect();
    let explicit: Vec<String> = on.into_iter().filter(|x| ids.contains(&x) && !off.contains(x)).collect();
    // Maven : 같은 pom 의 다른 프로필이 켜지면 activeByDefault 는 꺼진다
    let active = if explicit.is_empty() {
        profiles.iter().filter(|(id, def, _)| *def && !off.contains(id)).map(|(id, _, _)| id.clone()).collect()
    } else {
        explicit
    };
    let unsure = profiles.iter().filter(|(id, _, cond)| *cond && !active.contains(id) && !off.contains(id)).map(|(id, _, _)| id.clone()).collect();
    (active, unsure)
}

// java .properties — 「=」 · 「:」 · 공백 중 첫 구분자에서 나눈다. 값은 쓰지 않고 키만 쓰므로 이스케이프 · 이어 쓰기는 따지지 않는다.
fn read_props(path: &Path, into: &mut HashMap<String, String>) -> bool {
    let Ok(t) = fs::read(path) else { return false };
    for l in String::from_utf8_lossy(&t).lines() {
        let l = l.trim_start();
        if l.is_empty() || l.starts_with('#') || l.starts_with('!') {
            continue;
        }
        let at = l.find(|c: char| c == '=' || c == ':' || c.is_whitespace()).unwrap_or(l.len());
        let v = l[at..].trim_start_matches(|c: char| c == '=' || c == ':' || c.is_whitespace());
        into.insert(l[..at].to_string(), v.trim_end().to_string());
    }
    true
}

/// pom 을 읽어 걸러 낼 resource 와 값을 찾는 표를 만든다. 실행기가 올리는 resources 를 거르는 resource 도, 다른 폴더를 거르는 resource 도 없으면 None.
fn plan(project: &Path) -> Option<Plan> {
    let pom = fs::read_to_string(project.join("pom.xml")).ok()?;
    let doc = roxmltree::Document::parse(&pom).ok()?;
    let root = doc.root_element();
    let mut notes = Vec::new();

    let profile_nodes: Vec<Node> = child(root, "profiles").map(|p| p.children().filter(|c| c.has_tag_name("profile")).collect()).unwrap_or_default();
    let defs: Vec<(String, bool, bool)> = profile_nodes
        .iter()
        .map(|p| {
            let act = child(*p, "activation");
            let def = act.map_or(false, |a| text(child(a, "activeByDefault")) == "true");
            let cond = act.map_or(false, |a| a.children().any(|c| c.is_element() && !c.has_tag_name("activeByDefault")));
            (text(child(*p, "id")), def, cond)
        })
        .collect();
    let m2e = fs::read_to_string(project.join(M2E_PREFS)).unwrap_or_default();
    let (profiles, unsure) = active_profiles(&m2e, &defs);
    if !unsure.is_empty() {
        notes.push(format!("조건으로 켜지는 프로필 {} 은 끈 것으로 셌습니다", unsure.join(" · ")));
    }
    let active: Vec<Node> = profile_nodes.iter().zip(&defs).filter(|(_, (id, _, _))| profiles.contains(id)).map(|(n, _)| *n).collect();

    // 값 — 프로젝트 → 켠 프로필 순으로 덮는다. filters 파일은 그 뒤에 읽어 맨 아래에 깐다.
    let mut props = HashMap::new();
    props_of(root, &mut props);
    for p in &active {
        props_of(*p, &mut props);
    }
    // 프로필의 <build> 는 BuildBase 라 resources · filters 를 둘 수 있다
    let builds: Vec<Node> = child(root, "build").into_iter().chain(active.iter().filter_map(|p| child(*p, "build"))).collect();
    let mut vals = HashMap::new();
    for b in &builds {
        for f in list(*b, "filters", "filter") {
            let rel = interpolate(&f, &props);
            if rel.contains("${") || !read_props(&project.join(&rel), &mut vals) {
                notes.push(format!("filters 파일 {rel} 을 읽지 못했습니다"));
            }
        }
    }
    vals.extend(props);

    let mut filtered = Vec::new();
    let mut others = Vec::new();
    for b in &builds {
        let Some(rs) = child(*b, "resources") else { continue };
        for r in rs.children().filter(|c| c.has_tag_name("resource")) {
            if text(child(r, "filtering")) != "true" {
                continue;
            }
            let dir = norm_dir(&interpolate(&text(child(r, "directory")), &vals));
            // directory 를 적지 않으면 Maven 은 프로젝트 폴더로 본다 — 실행기가 올리는 폴더가 아니다
            if dir.eq_ignore_ascii_case(RESOURCES) {
                let pats = |outer: &str, inner: &str| -> Vec<String> { list(r, outer, inner).iter().map(|x| norm_pattern(x)).collect() };
                filtered.push(Resource { includes: pats("includes", "include"), excludes: pats("excludes", "exclude") });
            } else if !others.contains(&dir) {
                others.push(dir);
            }
        }
    }
    if filtered.is_empty() && others.is_empty() {
        return None;
    }
    if !others.is_empty() {
        let shown: Vec<String> = others.iter().map(|d| if d.is_empty() { "(프로젝트 폴더)".to_string() } else { d.clone() }).collect();
        notes.push(format!("{} 도 거르지만 실행기는 {RESOURCES} 만 올리므로 보지 않았습니다", shown.join(" · ")));
    }
    if child(root, "parent").is_some() {
        notes.push("상위 pom 에서 물려받은 속성은 보지 않았습니다".into());
    }
    if root.descendants().any(|n| n.has_tag_name("delimiters") || n.has_tag_name("escapeString") || n.has_tag_name("useDefaultDelimiters")) {
        notes.push("플러그인 설정의 구분자 · 이스케이프는 보지 않고 기본(${…} · @…@)으로 셌습니다".into());
    }
    let non_filtered = NON_FILTERED
        .iter()
        .map(|x| x.to_string())
        .chain(root.descendants().filter(|n| n.has_tag_name("nonFilteredFileExtension")).map(|n| text(Some(n)).to_ascii_lowercase()))
        .collect();
    Some(Plan { profiles, filtered, vals, non_filtered, notes })
}

/* ---------- 세기 ---------- */

/// ${key} · @key@ 의 key 들 (나온 차례대로 · 겹쳐도 그대로).
/// 구분자는 ASCII 이고 UTF-8 · MS949 의 둘째 바이트에는 나오지 않으므로 인코딩을 가리지 않고 바이트로 찾는다.
fn placeholders(b: &[u8]) -> Vec<String> {
    let key_ch = |c: u8| c.is_ascii_alphanumeric() || matches!(c, b'.' | b'_' | b'-');
    let mut out = Vec::new();
    let mut i = 0;
    while i < b.len() {
        let (open, close) = match b[i] {
            b'$' if b.get(i + 1) == Some(&b'{') => (2, b'}'),
            b'@' => (1, b'@'),
            _ => {
                i += 1;
                continue;
            }
        };
        let start = i + open;
        let mut j = start;
        while j < b.len() && key_ch(b[j]) && j - start < 200 {
            j += 1;
        }
        if j > start && b.get(j) == Some(&close) {
            out.push(String::from_utf8_lossy(&b[start..j]).into_owned());
            i = j + 1;
        } else {
            i += 1;
        }
    }
    out
}

fn walk(dir: &Path, rel: &str, out: &mut Vec<(String, std::path::PathBuf)>) {
    let Ok(rd) = fs::read_dir(dir) else { return };
    for e in rd.filter_map(Result::ok) {
        let name = e.file_name().to_string_lossy().to_string();
        // Maven 기본 제외(.svn · .git 등)와 편집기 임시 파일
        if name.starts_with('.') {
            continue;
        }
        let r = if rel.is_empty() { name } else { format!("{rel}/{name}") };
        let path = e.path();
        if path.is_dir() {
            walk(&path, &r, out);
        } else {
            out.push((r, path));
        }
    }
}

/// 필터링을 쓰지 않으면 None. 쓰면 src/main/resources 에서 Maven 이 값을 바꿔 넣을 자리를 센 결과.
pub fn scan(project: &Path) -> Option<Report> {
    let plan = plan(project)?;
    let mut files = Vec::new();
    if !plan.filtered.is_empty() {
        walk(&project.join(RESOURCES), "", &mut files);
    }
    files.sort();
    let mut rep = Report { profiles: plan.profiles.clone(), files: 0, hits: Vec::new(), places: 0, notes: plan.notes.clone() };
    for (rel, path) in files {
        if !plan.filters(&rel.to_ascii_lowercase()) || fs::metadata(&path).map_or(true, |m| m.len() > MAX_BYTES) {
            continue;
        }
        let Ok(bytes) = fs::read(&path) else { continue };
        // 바이너리 (jar · zip · 글꼴 등) — Maven 은 걸러서 망가뜨리지만 설정 값이 들어갈 자리는 아니다
        if bytes.iter().take(8192).any(|&c| c == 0) {
            continue;
        }
        rep.files += 1;
        let keys: Vec<String> = placeholders(&bytes).into_iter().filter(|k| plan.known(k)).collect();
        if keys.is_empty() {
            continue;
        }
        rep.places += keys.len();
        let mut uniq: Vec<String> = keys.into_iter().collect::<HashSet<_>>().into_iter().collect();
        uniq.sort();
        rep.hits.push((rel, uniq));
    }
    Some(rep)
}

/* ---------- 알림 ---------- */

pub struct Report {
    pub profiles: Vec<String>,            // 켠 프로필
    pub files: usize,                     // 걸러지는 파일 수 (src/main/resources)
    pub hits: Vec<(String, Vec<String>)>, // Maven 이 값을 바꿔 넣는 자리가 있는 파일 (resources 기준 경로) · 그 키
    pub places: usize,                    // 그런 자리 수
    pub notes: Vec<String>,               // 판단 못 한 것 · 보지 않은 것
}

impl Report {
    pub fn warn(&self) -> bool {
        !self.hits.is_empty()
    }

    fn profile_label(&self) -> String {
        if self.profiles.is_empty() {
            "프로필 없음".into()
        } else {
            format!("프로필 {}", self.profiles.join(" · "))
        }
    }

    fn hit_list(&self) -> Vec<String> {
        let mut v: Vec<String> = self.hits.iter().take(SHOWN).map(|(f, k)| format!("{f} ({})", k.join(" · "))).collect();
        if self.hits.len() > SHOWN {
            v.push(format!("외 {}개", self.hits.len() - SHOWN));
        }
        v
    }

    /// 「상태」 탭 서버 점검의 「리소스 필터링」 줄 — 여러 줄
    pub fn detail(&self) -> String {
        let mut lines = Vec::new();
        if self.warn() {
            lines.push(format!(
                "{} · 이클립스(Maven)가 값으로 바꾸는 자리표시자 {}곳 · 파일 {}개 — 실행기는 resources 를 그대로 읽어 이 자리에 글자가 그대로 들어갑니다. 값을 파일에 직접 적거나 JVM 인자(-D…)로 받게 하세요",
                self.profile_label(),
                self.places,
                self.hits.len()
            ));
            lines.extend(self.hit_list().into_iter().map(|h| format!("· {h}")));
        } else {
            lines.push(format!("필터링 켜짐 · {} · 걸러지는 파일 {}개 · 값이 바뀌는 자리표시자 없음", self.profile_label(), self.files));
        }
        lines.extend(self.notes.iter().map(|n| format!("판단 못 함 : {n}")));
        lines.join("\n")
    }

    /// 「시작하기」 서버 줄 아래 경고 한 줄 — 바뀌는 자리가 없으면 없다
    pub fn warning(&self) -> Option<String> {
        self.warn().then(|| {
            format!(
                "pom.xml 리소스 필터링으로 이클립스(Maven)가 값을 바꿔 넣는 자리표시자가 {}곳 있습니다 ({}) — 실행기는 src/main/resources 를 그대로 읽어 이 자리에 글자가 그대로 들어갑니다",
                self.places,
                self.hit_list().join(" · ")
            )
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn project(name: &str, files: &[(&str, &str)]) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("tomcatrun-filtering-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        for (rel, body) in files {
            let f = dir.join(rel);
            fs::create_dir_all(f.parent().unwrap()).unwrap();
            fs::write(f, body).unwrap();
        }
        dir
    }

    const POM: &str = r#"<project xmlns="http://maven.apache.org/POM/4.0.0">
  <properties><app.name>shop</app.name><db.pool>10</db.pool></properties>
  <build>
    <filters><filter>${basedir}/src/main/filters/${env}.properties</filter></filters>
    <resources>
      <resource><directory>src/main/resources</directory><filtering>true</filtering>
        <includes><include>**/*.properties</include><include>spring/</include><include>*.png</include></includes>
        <excludes><exclude>skip/**</exclude></excludes></resource>
      <resource><directory>src/main/resources</directory><filtering>false</filtering></resource>
    </resources>
  </build>
  <profiles>
    <profile><id>local</id><activation><activeByDefault>true</activeByDefault></activation><properties><env>local</env><db.pool>2</db.pool></properties></profile>
    <profile><id>dev</id><properties><env>dev</env></properties></profile>
    <profile><id>ci</id><activation><property><name>ci</name></property></activation></profile>
  </profiles>
</project>"#;

    #[test]
    fn 켠_프로필은_m2e_설정을_먼저_따른다() {
        let defs = vec![("local".to_string(), true, false), ("dev".to_string(), false, false), ("ci".to_string(), false, true)];
        assert_eq!(active_profiles("", &defs), (vec!["local".to_string()], vec!["ci".to_string()]));
        assert_eq!(active_profiles("eclipse.preferences.version=1\nactiveProfiles=dev\n", &defs).0, ["dev"]);
        assert_eq!(active_profiles("activeProfiles=dev, !ci\n", &defs).1, Vec::<String>::new());
        // 끈 것만 적으면 activeByDefault 가 남고, 그것까지 끄면 아무것도 켜지지 않는다
        assert_eq!(active_profiles("activeProfiles=!dev\n", &defs).0, ["local"]);
        assert!(active_profiles("activeProfiles=!local\n", &defs).0.is_empty());
        // pom 에 없는 프로필(settings.xml 쪽)은 이 pom 의 activeByDefault 를 끄지 않는다
        assert_eq!(active_profiles("activeProfiles=nexus\n", &defs).0, ["local"]);
    }

    #[test]
    fn 자리표시자는_두_꼴을_바이트로_찾는다() {
        let b = "a=${app.name}\nb=@db.pool@\nmail=x@y.com\nc=${ 공백 }\n한글=${한}\nd=${env.X_Y}".as_bytes();
        assert_eq!(placeholders(b), ["app.name", "db.pool", "env.X_Y"]);
        // MS949 로 적힌 한글 사이에서도 찾는다
        let ms949 = [0xC7, 0xD1, b'=', b'$', b'{', b'a', b'}', 0xB1, 0xDB];
        assert_eq!(placeholders(&ms949), ["a"]);
    }

    #[test]
    fn maven_이_아는_키만_센다() {
        let dir = project(
            "scan",
            &[
                ("pom.xml", POM),
                (".settings/org.eclipse.m2e.core.prefs", "activeProfiles=dev\n"),
                ("src/main/filters/dev.properties", "jdbc.url = jdbc:oracle:thin:@dev\n# 주석\nmail.host: smtp\n"),
                ("src/main/resources/app.properties", "name=${app.name}\npool=${db.pool}\nurl=${jdbc.url}\nhost=@mail.host@\nspring=${spring.only}\nhome=${user.home}\n"),
                ("src/main/resources/spring/context.xml", "<bean p:url=\"${db.url}\" p:v=\"${project.version}\"/>"),
                ("src/main/resources/sqlmap/a.xml", "${app.name}"),
                ("src/main/resources/skip/b.properties", "${app.name}"),
                ("src/main/resources/logo.png", "${app.name}"),
                ("src/main/resources/c.properties", "plain=1"),
            ],
        );
        let r = scan(&dir).unwrap();
        assert_eq!(r.profiles, ["dev"]);
        // includes 밖(sqlmap) · excludes(skip) · 이미지(png)는 걸러지지 않는다 — app · c.properties 와 spring/ 아래
        assert_eq!(r.files, 3);
        assert_eq!(r.hits.len(), 2);
        assert_eq!(r.hits[0], ("app.properties".to_string(), ["app.name", "db.pool", "jdbc.url", "mail.host", "user.home"].map(String::from).to_vec()));
        // Spring 이 채우는 ${db.url} 은 세지 않는다
        assert_eq!(r.hits[1], ("spring/context.xml".to_string(), vec!["project.version".to_string()]));
        assert_eq!(r.places, 6);
        assert!(r.warn() && r.detail().contains("프로필 dev · 이클립스(Maven)가 값으로 바꾸는 자리표시자 6곳 · 파일 2개"));
        assert!(r.warning().unwrap().contains("app.properties (app.name · db.pool"));
        assert_eq!(r.notes, ["조건으로 켜지는 프로필 ci 은 끈 것으로 셌습니다"]);
        assert!(r.detail().ends_with("판단 못 함 : 조건으로 켜지는 프로필 ci 은 끈 것으로 셌습니다"));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn 필터링이_없거나_바뀌는_자리가_없으면() {
        let none = project("none", &[("pom.xml", "<project><build><resources><resource><directory>src/main/resources</directory></resource></resources></build></project>")]);
        assert!(scan(&none).is_none());
        // 꺼진 프로필 안의 filtering 은 쓰지 않는 것이다
        let off = project("off", &[("pom.xml", "<project><profiles><profile><id>p</id><build><resources><resource><directory>src/main/resources</directory><filtering>true</filtering></resource></resources></build></profile></profiles></project>")]);
        assert!(scan(&off).is_none());
        let quiet = project(
            "quiet",
            &[
                ("pom.xml", "<project><parent><artifactId>base</artifactId></parent><build><resources><resource><directory>${project.basedir}/src/main/resources</directory><filtering>true</filtering></resource><resource><directory>src/main/resources-dev</directory><filtering>true</filtering></resource></resources></build></project>"),
                ("src/main/resources/a.properties", "url=${db.url}"),
            ],
        );
        let r = scan(&quiet).unwrap();
        assert!(!r.warn() && r.warning().is_none() && r.files == 1);
        assert!(r.detail().starts_with("필터링 켜짐 · 프로필 없음 · 걸러지는 파일 1개"));
        assert!(r.notes.iter().any(|n| n.contains("src/main/resources-dev 도")) && r.notes.iter().any(|n| n.contains("상위 pom")));
        for d in [none, off, quiet] {
            let _ = fs::remove_dir_all(d);
        }
    }
}
