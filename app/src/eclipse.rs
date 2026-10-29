// 이클립스 설정 읽기 — 「시작하기」 에서 이클립스 작업 공간의 톰캣 서버를 그대로 가져온다.
//
// 이클립스(WTP)가 남기는 것 (모두 읽기만 한다)
//   <작업 공간>/.metadata/.plugins/org.eclipse.wst.server.core/servers.xml   서버 이름 · 모듈(프로젝트) · 설정 폴더 · 런타임
//   <작업 공간>/Servers/<설정 폴더>/server.xml                               종료 · HTTP · AJP 포트 · HTTP maxParameterCount · 그 모듈의 접속 경로 · JNDI (GlobalNamingResources · 그 모듈의 Context)
//   <작업 공간>/Servers/<설정 폴더>/context.xml                              JNDI (모든 앱에 붙는 기본 Context)
//   <프로젝트>/src/main/webapp/META-INF/context.xml                          JNDI — 실행기는 Context 를 server.xml 에 적어 톰캣이 이 파일을 읽지 않는다
//   <작업 공간>/.metadata/.plugins/org.eclipse.wst.server.core/tmp-data.xml  서버마다 배포 폴더(tmpN)
//   tmpN/wtpwebapps/<프로젝트>/WEB-INF/lib                                    이클립스가 배포한 의존 jar — Maven 없이 deps 를 채운다
//   <작업 공간>/.metadata/.plugins/org.eclipse.core.runtime/.settings/org.eclipse.wst.server.core.prefs   런타임(톰캣) 폴더
//   <작업 공간>/.metadata/.plugins/org.eclipse.debug.core/.launches/<서버 id>.launch   사용자가 더한 JVM 인자 · 마지막으로 띄운 JRE 이름
//   <작업 공간>/.metadata/.plugins/org.eclipse.core.runtime/.settings/org.eclipse.jdt.launching.prefs   설치된 JRE 목록 · 작업 공간 기본 JRE
//   <작업 공간>/<프로젝트> 또는 .projects/<프로젝트>/.location                  프로젝트 폴더
//
// 서버의 JRE 는 launch 의 JRE_CONTAINER → 서버 런타임의 vm-install-id → 작업 공간 기본 JRE 순서로 찾는다.
// 2026.10.29 : 실행기는 핫리로드를 끈 기동에도 함께 까는 8u181 을 써서, 더 새 JDK 를 쓰던 프로젝트가 이 실행기에서만 SSL · 클래스 버전 오류를 냈다.
//
// 실행기와 다르게 도는 것 — 경고로만 알린다 (고쳐 주지 않는다)
//   Servers/<설정 폴더>/server.xml · context.xml   JNDI 자원 · HTTP Connector 속성 (접속 경로 · maxParameterCount 는 서버 설정으로 옮겨 오므로 견주지 않는다)
//   .launch                                         Classpath 탭에 더한 항목 · Environment 탭의 환경 변수
//   <프로젝트>/.settings/org.eclipse.core.resources.prefs   소스 인코딩 (없으면 작업 공간 기본) — 가져올 때 서버 설정으로 옮기고, 서버 설정과 다를 때만 경고한다
//   <프로젝트>/.settings/org.eclipse.jdt.core.prefs         compliance · 매개변수 이름 남기기(-parameters — 인코딩과 같이 옮긴다)
//   <프로젝트>/.settings/org.eclipse.wst.common.component   배포 폴더 · 함께 배포하는 다른 프로젝트
//   <프로젝트>/pom.xml                                      리소스 필터링 (filtering.rs 가 센다 · 여기서는 「시작하기」 목록에 붙이기만)
//
// 작업 공간은 흔한 자리(사용자 폴더 · 각 드라이브 맨 위 한 단계)에서 찾는다. 못 찾으면 사용자가 폴더를 고른다.

use std::fs;
use std::path::{Path, PathBuf};

use serde::Serialize;

const WTP: &str = ".metadata/.plugins/org.eclipse.wst.server.core";

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EServer {
    pub id: String,           // 이클립스 서버 id (Tomcat v8.5 Server at localhost)
    pub name: String,         // 서버 이름 (사용자가 바꾼 것)
    pub project: String,      // 프로젝트 폴더 (못 찾으면 빈 값)
    pub tomcat_home: String,  // 런타임 폴더 (못 찾으면 빈 값)
    pub http_port: u16,
    pub ajp_port: u16,
    pub shutdown_port: u16,
    pub context_path: String,     // 그 모듈의 <Context path> — 루트는 "", 그 밖은 "/이름"
    pub max_parameter_count: i32, // HTTP Connector 의 maxParameterCount — 적지 않았으면 톰캣 기본 10000
    pub lib_dir: String,      // 이클립스가 배포한 WEB-INF/lib (없으면 빈 값)
    pub jars: usize,          // 그중 프로젝트 WEB-INF/lib 에 없는 것 = deps 로 옮길 것
    pub jvm_args: Vec<String>,
    pub naming: Vec<NamingBrief>,   // 화면에 보일 JNDI 요약 — 속성 값(DB 계정 · 비밀번호)은 싣지 않는다
    #[serde(skip)]
    pub naming_full: Vec<NamingEntry>, // 옮길 때 쓰는 원본 — 화면으로 내보내지 않는다
    pub warnings: Vec<String>,      // 실행기와 다르게 도는 것 전부 — 「시작하기」 목록에 보인다
    #[serde(skip)]
    pub server_warnings: Vec<String>, // 그중 작업 공간에서만 읽을 수 있는 것 — 가져올 때 서버 설정에 남긴다 ({base} 는 그 서버의 CATALINA_BASE)
    pub java: String,         // 이클립스가 이 서버를 띄우던 java.exe (못 찾으면 빈 값)
    pub java_name: String,    // 이클립스에 등록된 JRE 이름 (jdk-17 등)
    pub java_version: String, // 그 JRE 의 판 (17.0.8 · 8u181) — 실행기가 채운다 (eclipse_scan)
    pub encoding: String,     // 자바 소스 인코딩 — 프로젝트 → 작업 공간 기본 순. UTF-8 이면 빈 값 (ServerCfg.encoding 과 같은 꼴)
    #[serde(skip)]
    pub encoding_unset: bool, // 프로젝트에도 작업 공간에도 인코딩을 정하지 않았다 — 가져올 때 로그에 한 줄 남긴다
    pub parameters: bool,     // 매개변수 이름을 남기고 컴파일한다 (-parameters)
}

/// 이클립스 서버 설정(Servers/<설정 폴더> · .launch)에서 실행기와 다른 것. 작업 공간이 있어야 읽으므로 가져올 때 한 번 읽는다.
#[derive(Clone, Default)]
pub struct ServerDiff {
    pub connector: Vec<(String, String, String)>, // HTTP Connector 속성 중 실행기와 다른 것 (이름 · 이클립스 값 · 실행기 값)
    pub lost_links: Vec<String>,           // 가리키는 Global 항목이 없어 옮길 수 없는 JNDI 링크
    pub classpath: Vec<String>,            // .launch Classpath 탭에 더한 항목
    pub env: Vec<String>,                  // .launch Environment 탭의 환경 변수 이름
}

/// 프로젝트 폴더의 이클립스 설정(.settings)에서 실행기와 다른 것. 프로젝트만 있으면 읽으므로 점검할 때마다 다시 읽는다.
#[derive(Clone, Default)]
pub struct ProjectSettings {
    pub encoding: String,           // 프로젝트에 정한 소스 인코딩 — src/main/java 폴더 → 프로젝트 전체 순 (비면 작업 공간 기본을 따른다)
    pub ws_encoding: Option<String>, // 프로젝트에 없을 때 작업 공간 기본 — 작업 공간을 알 때만 채운다 (거기도 없으면 빈 값 = 이클립스 판 · Windows 기본)
    pub file_encodings: Vec<String>, // src/main/java 아래 파일 · 폴더 하나만 따로 정한 인코딩 — 실행기는 소스 폴더 전체를 한 인코딩으로 컴파일한다
    pub compliance: String,         // 비면 작업 공간 기본 (프로젝트에 따로 정하지 않았다)
    pub method_parameters: bool,    // 매개변수 이름을 남기고 컴파일한다 (javac -parameters)
    pub deploy_extra: Vec<String>,  // src/main/webapp · resources · java 밖에서 배포하는 폴더
    pub dependent: Vec<String>,     // 함께 배포하는 다른 프로젝트
}

/// JNDI 항목 하나 (Resource · Environment). ResourceLink 는 가리키는 Global 항목의 속성에 링크 이름을 붙여 Resource 로 바꿔 둔다.
/// Global 항목을 찾지 못한 링크는 kind 가 「ResourceLink」 로 남고 옮기지 않는다.
#[derive(Clone)]
pub struct NamingEntry {
    pub kind: String,
    pub name: String,
    pub attrs: Vec<(String, String)>, // 원래 순서 그대로 (name 포함)
    pub from: String,                 // server.xml · META-INF/context.xml · context.xml
}

impl NamingEntry {
    pub fn movable(&self) -> bool {
        self.kind != "ResourceLink"
    }
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NamingBrief {
    pub kind: String,
    pub name: String,
    #[serde(rename = "type")]
    pub type_: String,
    pub from: String,
    pub secret: bool,   // 비밀번호 속성이 있다
    pub missing: bool,  // 가리키는 Global 항목이 없는 링크 — 옮기지 않는다
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Workspace {
    pub path: String,
    pub servers: Vec<EServer>,
}

fn p(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

pub fn is_workspace(dir: &Path) -> bool {
    dir.join(WTP).join("servers.xml").is_file()
}

/// 흔한 자리에서 작업 공간을 찾는다 — 사용자 폴더와 각 드라이브의 맨 위, 그 한 단계 아래. extra(지금 설정된 프로젝트의 부모 등)도 본다.
pub fn find_workspaces(extra: &[PathBuf]) -> Vec<PathBuf> {
    let mut roots: Vec<PathBuf> = extra.to_vec();
    if let Some(home) = std::env::var_os("USERPROFILE").map(PathBuf::from) {
        roots.push(home.clone());
        roots.push(home.join("Documents"));
    }
    for d in b'C'..=b'Z' {
        let root = PathBuf::from(format!("{}:\\", d as char));
        if root.is_dir() {
            roots.push(root);
        }
    }
    let mut out: Vec<PathBuf> = Vec::new();
    let mut add = |d: PathBuf| {
        if is_workspace(&d) && !out.iter().any(|o| p(o).eq_ignore_ascii_case(&p(&d))) {
            out.push(d);
        }
    };
    for r in roots {
        add(r.clone());
        for e in fs::read_dir(&r).into_iter().flatten().flatten() {
            let d = e.path();
            let name = e.file_name().to_string_lossy().to_string();
            // 시스템 폴더는 건너뛴다 (느리고 권한이 없다)
            if d.is_dir() && !name.starts_with('$') && !name.starts_with('.') && !["Windows", "Program Files", "Program Files (x86)", "ProgramData", "AppData"].contains(&name.as_str()) {
                add(d);
            }
        }
    }
    out
}

fn parse(text: &str) -> Option<roxmltree::Document<'_>> {
    let opt = roxmltree::ParsingOptions { allow_dtd: true, ..Default::default() };
    roxmltree::Document::parse_with_options(text, opt).ok()
}

// .launch 의 VM 인자에서 이클립스가 넣는 것(catalina.* · wtp.* · endorsed)을 빼고 사용자가 더한 것만 남긴다
fn user_jvm_args(launch: &str) -> Vec<String> {
    let Some(doc) = parse(launch) else { return vec![] };
    let Some(v) = doc
        .descendants()
        .find(|n| n.attribute("key") == Some("org.eclipse.jdt.launching.VM_ARGUMENTS"))
        .and_then(|n| n.attribute("value"))
    else {
        return vec![];
    };
    // 따옴표 안의 공백은 나누지 않는다
    let (mut out, mut cur, mut q) = (Vec::new(), String::new(), false);
    for ch in v.chars() {
        match ch {
            '"' => q = !q,
            c if c.is_whitespace() && !q => {
                if !cur.is_empty() {
                    out.push(std::mem::take(&mut cur));
                }
            }
            c => cur.push(c),
        }
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out.into_iter()
        .filter(|a| !["-Dcatalina.base=", "-Dcatalina.home=", "-Dwtp.deploy=", "-Djava.endorsed.dirs="].iter().any(|x| a.starts_with(x)))
        .collect()
}

// 프로젝트 폴더 — 작업 공간 안이면 그 자리, 밖에 있으면 .location 파일(바이너리) 안의 「URI//file:/…」
fn project_dir(ws: &Path, name: &str) -> Option<PathBuf> {
    let inside = ws.join(name);
    if inside.join("src/main/webapp").is_dir() || inside.join(".project").is_file() {
        return Some(inside);
    }
    let loc = fs::read(ws.join(".metadata/.plugins/org.eclipse.core.resources/.projects").join(name).join(".location")).ok()?;
    let text = String::from_utf8_lossy(&loc);
    let at = text.find("file:/")?;
    let rest: String = text[at + 6..].chars().take_while(|c| !c.is_control()).collect();
    let path = PathBuf::from(rest.trim_start_matches('/').replace("%20", " "));
    path.is_dir().then_some(path)
}

// .prefs 파일 안에 한 줄로 들어 있는 XML 값 (runtimes= · PREF_VM_XML=)
fn pref_xml(ws: &Path, file: &str, key: &str) -> String {
    let text = fs::read_to_string(ws.join(".metadata/.plugins/org.eclipse.core.runtime/.settings").join(file)).unwrap_or_default();
    prefs(&text).into_iter().find(|(k, _)| k == key).map(|(_, v)| v).unwrap_or_default()
}

fn runtime_home(ws: &Path, runtime_id: &str) -> String {
    let xml = pref_xml(ws, "org.eclipse.wst.server.core.prefs", "runtimes");
    let Some(doc) = parse(&xml) else { return String::new() };
    doc.descendants()
        .find(|n| n.has_tag_name("runtime") && n.attribute("id") == Some(runtime_id))
        .and_then(|n| n.attribute("location"))
        .map(|l| l.replace('\\', "/"))
        .filter(|l| Path::new(l).join("bin/bootstrap.jar").is_file())
        .unwrap_or_default()
}

// 서버 런타임에 고른 JRE 의 id — 비어 있으면 작업 공간 기본 JRE 를 쓴다
fn runtime_vm_id(ws: &Path, runtime_id: &str) -> String {
    let xml = pref_xml(ws, "org.eclipse.wst.server.core.prefs", "runtimes");
    let Some(doc) = parse(&xml) else { return String::new() };
    doc.descendants()
        .find(|n| n.has_tag_name("runtime") && n.attribute("id") == Some(runtime_id))
        .and_then(|n| n.attribute("vm-install-id"))
        .unwrap_or("")
        .to_string()
}

// 「57,org…StandardVMType13,1700000000000」 꼴의 복합 id — 「길이,문자열」 이 이어진다. 마지막 조각이 JRE id
fn composite_last(id: &str) -> String {
    let (mut rest, mut last) = (id, String::new());
    while let Some(comma) = rest.find(',') {
        let Ok(n) = rest[..comma].parse::<usize>() else { break };
        let body = &rest[comma + 1..];
        let Some(part) = body.get(..n) else { break };
        last = part.to_string();
        rest = &body[n..];
    }
    last
}

/// launch 의 JRE 이름 → 런타임의 JRE id → 작업 공간 기본 JRE 순서로 찾아 (java.exe, 이름) 을 돌려준다.
/// launch 가 실행 환경(JavaSE-17 등)을 가리키면 이름이 맞는 JRE 가 없어 다음 단계로 넘어간다.
fn server_java(ws: &Path, launch: &str, runtime_id: &str) -> Option<(String, String)> {
    let xml = pref_xml(ws, "org.eclipse.jdt.launching.prefs", "org.eclipse.jdt.launching.PREF_VM_XML");
    let doc = parse(&xml)?;
    let vms: Vec<(String, String, String)> = doc
        .descendants()
        .filter(|n| n.has_tag_name("vm"))
        .map(|n| (n.attribute("id").unwrap_or("").to_string(), n.attribute("name").unwrap_or("").to_string(), n.attribute("path").unwrap_or("").replace('\\', "/")))
        .collect();
    let exe = |home: &str| {
        let j = Path::new(home).join("bin/java.exe");
        j.is_file().then(|| p(&j))
    };
    let container = parse(launch)
        .and_then(|d| d.descendants().find(|n| n.attribute("key") == Some("org.eclipse.jdt.launching.JRE_CONTAINER")).and_then(|n| n.attribute("value")).map(String::from))
        .unwrap_or_default();
    let by_name = container.rsplit('/').next().map(|s| s.replace("%20", " ")).filter(|s| !s.is_empty());
    let by_runtime = Some(runtime_vm_id(ws, runtime_id)).filter(|s| !s.is_empty());
    let by_default = doc.root_element().attribute("defaultVM").map(composite_last).filter(|s| !s.is_empty());
    let found = by_name
        .and_then(|name| vms.iter().find(|v| v.1 == name))
        .or_else(|| by_runtime.and_then(|id| vms.iter().find(|v| v.0 == id)))
        .or_else(|| by_default.and_then(|id| vms.iter().find(|v| v.0 == id)))?;
    Some((exe(&found.2)?, found.1.clone()))
}

fn jar_names(dir: &Path) -> Vec<String> {
    fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.file_name().to_string_lossy().to_string())
        .filter(|n| n.to_ascii_lowercase().ends_with(".jar"))
        .collect()
}

/// 이클립스 배포본에만 있는 jar (프로젝트의 WEB-INF/lib 에 이미 있는 것은 docBase 로 들어오므로 뺀다)
fn extra_jars(lib: &Path, project: &str) -> Vec<String> {
    let own: Vec<String> = jar_names(&Path::new(project).join("src/main/webapp/WEB-INF/lib")).iter().map(|n| n.to_ascii_lowercase()).collect();
    jar_names(lib).into_iter().filter(|n| !own.contains(&n.to_ascii_lowercase())).collect()
}

fn attrs_of(n: roxmltree::Node) -> Vec<(String, String)> {
    n.attributes().map(|a| (a.name().to_string(), a.value().to_string())).collect()
}

// Context 의 자식 Resource · Environment · ResourceLink 를 모은다. 이름이 이미 있으면 앞의 것이 이긴다.
fn collect_naming(ctx: roxmltree::Node, global: &[NamingEntry], from: &str, out: &mut Vec<NamingEntry>) {
    for n in ctx.children().filter(|n| n.is_element()) {
        let kind = n.tag_name().name();
        if !["Resource", "Environment", "ResourceLink"].contains(&kind) {
            continue;
        }
        let name = n.attribute("name").unwrap_or("").to_string();
        if name.is_empty() || out.iter().any(|e| e.name == name) {
            continue;
        }
        let entry = if kind == "ResourceLink" {
            let target = n.attribute("global").unwrap_or("");
            match global.iter().find(|g| g.name == target) {
                // Global 항목의 속성을 링크 이름으로 — 앱은 java:comp/env/<링크 이름> 으로 찾는다
                Some(g) => NamingEntry {
                    kind: g.kind.clone(),
                    attrs: g.attrs.iter().map(|(k, v)| (k.clone(), if k == "name" { name.clone() } else { v.clone() })).collect(),
                    name,
                    from: format!("{from} (Global {target})"),
                },
                None => NamingEntry { kind: kind.into(), attrs: attrs_of(n), name, from: format!("{from} (Global {target} 없음)") },
            }
        } else {
            NamingEntry { kind: kind.into(), attrs: attrs_of(n), name, from: from.into() }
        };
        out.push(entry);
    }
}

/// 이클립스로 띄울 때 그 앱이 보던 JNDI 항목. 이클립스와 같은 순서로 본다 —
/// server.xml 의 그 모듈 Context → 프로젝트 META-INF/context.xml → 서버 설정 폴더의 context.xml.
/// GlobalNamingResources 의 항목은 ResourceLink 가 가리킬 때만 들어온다 (이클립스 기본값 UserDatabase 등은 빠진다).
pub fn naming_entries(server_xml: &str, context_xml: &str, meta_inf_xml: &str, module: &str) -> Vec<NamingEntry> {
    let mut out = Vec::new();
    let mut global = Vec::new();
    let sdoc = parse(server_xml);
    if let Some(d) = &sdoc {
        if let Some(g) = d.descendants().find(|n| n.has_tag_name("GlobalNamingResources")) {
            for n in g.children().filter(|n| n.has_tag_name("Resource") || n.has_tag_name("Environment")) {
                let name = n.attribute("name").unwrap_or("").to_string();
                global.push(NamingEntry { kind: n.tag_name().name().into(), attrs: attrs_of(n), name, from: "server.xml".into() });
            }
        }
        // 이클립스가 적는 Context 는 source="org.eclipse.jst.jee.server:<모듈>" 이다
        let mine = |c: &roxmltree::Node| {
            c.attribute("source").and_then(|s| s.rsplit(':').next()) == Some(module) || c.attribute("docBase") == Some(module)
        };
        if let Some(ctx) = d.descendants().find(|n| n.has_tag_name("Context") && mine(n)) {
            collect_naming(ctx, &global, "server.xml", &mut out);
        }
    }
    for (text, from) in [(meta_inf_xml, "META-INF/context.xml"), (context_xml, "context.xml")] {
        if let Some(d) = parse(text) {
            if d.root_element().has_tag_name("Context") {
                collect_naming(d.root_element(), &global, from, &mut out);
            }
        }
    }
    out
}

fn brief(e: &NamingEntry) -> NamingBrief {
    let get = |k: &str| e.attrs.iter().find(|(n, _)| n == k).map(|(_, v)| v.clone()).unwrap_or_default();
    NamingBrief {
        kind: e.kind.clone(),
        name: e.name.clone(),
        type_: get("type"),
        from: e.from.clone(),
        secret: e.attrs.iter().any(|(k, _)| k.to_ascii_lowercase().contains("password")),
        missing: !e.movable(),
    }
}

/* ---------- 실행기와 다른 점 (경고로만 알린다) ---------- */

// 실행기 server.xml 의 HTTP Connector 와 견줄 속성과 톰캣 8.5 기본값 — 속성을 적지 않으면 기본값으로 돈다
// 2026.10.29 : maxParameterCount 를 뺐다. 서버 설정으로 옮겨 와 가져오면 이클립스와 같아진다 (server_values).
const CONNECTOR_ATTRS: [(&str, &str); 3] = [("maxPostSize", "2097152"), ("URIEncoding", "UTF-8"), ("relaxedQueryChars", "")];
// 톰캣 8.5 기본 — 이클립스 server.xml 의 HTTP Connector 에 maxParameterCount 가 없으면 이 값으로 돈다
const TOMCAT_MAX_PARAMS: i32 = 10000;
// 서버를 새로 더할 때 복사하는 원본 (main.rs ensure_base) — 값을 따로 적어 두면 원본을 고쳤을 때 어긋난다
const LAUNCHER_SERVER_XML: &str = include_str!("../../base/conf/server.xml");

// 포트를 읽을 때와 같은 규칙 — AJP · SSL · 8443 이 아닌 첫 Connector
fn http_connector<'a, 'i>(d: &'a roxmltree::Document<'i>) -> Option<roxmltree::Node<'a, 'i>> {
    d.descendants().find(|c| {
        c.has_tag_name("Connector")
            && !c.attribute("protocol").unwrap_or("HTTP/1.1").starts_with("AJP")
            && c.attribute("SSLEnabled") != Some("true")
            && c.attribute("port") != Some("8443")
    })
}

// 자바 properties 이스케이프 — 「\=」 「\:」 「\\」 는 그 글자, 「\r\n」 은 줄바꿈, 「\u」 뒤 네 자리는 유니코드 (한글 JRE 경로)
// 2026.10.29 : 줄바꿈 · \u 를 더했다. runtimes · PREF_VM_XML 은 XML 이 한 줄에 「\r\n」 으로 들어 있어, 「rn」 이 남으면 XML 로 읽히지 않는다.
fn unescape(s: &str) -> String {
    let (mut out, mut it) = (String::new(), s.chars());
    while let Some(c) = it.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match it.next() {
            Some('n') => out.push('\n'),
            Some('r') => out.push('\r'),
            Some('t') => out.push('\t'),
            Some('u') => {
                let hex: String = it.by_ref().take(4).collect();
                out.extend(u32::from_str_radix(&hex, 16).ok().and_then(char::from_u32));
            }
            Some(x) => out.push(x),
            None => {}
        }
    }
    out
}

// 이클립스 .prefs (java properties) — 키 안의 「\=」 「\:」 는 글자이므로 이스케이프되지 않은 첫 = 에서 나눈다
fn prefs(text: &str) -> Vec<(String, String)> {
    text.lines()
        .filter(|l| !l.trim_start().starts_with('#'))
        .filter_map(|l| {
            let mut esc = false;
            let at = l
                .char_indices()
                .find(|&(_, c)| {
                    let hit = c == '=' && !esc;
                    esc = c == '\\' && !esc;
                    hit
                })
                .map(|(i, _)| i)?;
            Some((unescape(l[..at].trim()), unescape(l[at + 1..].trim())))
        })
        .collect()
}

fn utf8(enc: &str) -> bool {
    enc.eq_ignore_ascii_case("UTF-8") || enc.eq_ignore_ascii_case("UTF8")
}

/// 서버 설정에 둘 꼴 — 앞뒤 공백을 빼고, UTF-8 은 빈 값(기본)으로
pub fn clean_encoding(enc: &str) -> String {
    let e = enc.trim();
    if utf8(e) { String::new() } else { e.to_string() }
}

/// 같은 문자 집합인가 — 빈 값은 UTF-8, MS949 · CP949 · x-windows-949 는 이름만 다르다
pub fn same_encoding(a: &str, b: &str) -> bool {
    let key = |e: &str| {
        let k: String = clean_encoding(e).chars().filter(|c| *c != '-' && *c != '_').collect::<String>().to_ascii_lowercase();
        if ["cp949", "xwindows949", "windows949"].contains(&k.as_str()) { "ms949".to_string() } else { k }
    };
    key(a) == key(b)
}

fn shown_encoding(enc: &str) -> &str {
    if enc.trim().is_empty() { "UTF-8" } else { enc.trim() }
}

// .launch 의 Classpath 탭에 더한 항목 · Environment 탭의 변수 이름. 이클립스가 넣는 기본(JRE · bootstrap · tomcat-juli · tools.jar)은 뺀다.
fn launch_extras(launch: &str) -> (Vec<String>, Vec<String>) {
    let Some(doc) = parse(launch) else { return (vec![], vec![]) };
    let mut cp = Vec::new();
    if let Some(list) = doc.descendants().find(|n| n.attribute("key") == Some("org.eclipse.jdt.launching.CLASSPATH")) {
        // 항목마다 value 에 <runtimeClasspathEntry …/> 가 문자열로 들어 있다
        for v in list.children().filter_map(|n| n.attribute("value")) {
            let Some(d) = parse(v) else { continue };
            let r = d.root_element();
            if r.attribute("containerPath").map_or(false, |c| c.starts_with("org.eclipse.jdt.launching.JRE_CONTAINER")) {
                continue;
            }
            let archive = r.attribute("externalArchive").or(r.attribute("internalArchive")).map(|a| a.rsplit(['/', '\\']).next().unwrap_or(a).to_string());
            if archive.as_deref().map_or(false, |a| ["bootstrap.jar", "tomcat-juli.jar", "tools.jar"].contains(&a.to_ascii_lowercase().as_str())) {
                continue;
            }
            let label = archive
                .or(r.attribute("projectName").map(|x| format!("프로젝트 {x}")))
                .or(r.attribute("containerPath").map(String::from))
                .or(r.descendants().find_map(|m| m.attribute("project")).map(|x| format!("프로젝트 {x}")));
            if let Some(l) = label.filter(|l| !cp.contains(l)) {
                cp.push(l);
            }
        }
    }
    let env = doc
        .descendants()
        .find(|n| n.attribute("key") == Some("org.eclipse.debug.core.environmentVariables"))
        .map(|m| m.children().filter_map(|e| e.attribute("key")).map(String::from).collect())
        .unwrap_or_default();
    (cp, env)
}

/// 이클립스 서버 설정에서 실행기와 다른 것 — server.xml · .launch 원문과 이미 읽은 JNDI 항목으로
pub fn server_diff(server_xml: &str, launch: &str, naming: &[NamingEntry]) -> ServerDiff {
    let mut d = ServerDiff::default();
    if let Some(doc) = parse(server_xml) {
        let mine = parse(LAUNCHER_SERVER_XML);
        let ours = mine.as_ref().and_then(http_connector);
        if let Some(c) = http_connector(&doc) {
            for (a, def) in CONNECTOR_ATTRS {
                let theirs = c.attribute(a).unwrap_or(def);
                let ours = ours.and_then(|o| o.attribute(a)).unwrap_or(def);
                if !theirs.eq_ignore_ascii_case(ours) {
                    d.connector.push((a.to_string(), theirs.to_string(), ours.to_string()));
                }
            }
        }
    }
    d.lost_links = naming.iter().filter(|e| !e.movable()).map(|e| e.name.clone()).collect();
    (d.classpath, d.env) = launch_extras(launch);
    d
}

/// 실행기 서버 설정으로 옮겨 오는 값 — (그 모듈의 접속 경로, HTTP Connector 의 maxParameterCount).
/// 접속 경로는 루트면 "", 그 밖은 끝 / 없이 "/이름". 숫자가 아닌 maxParameterCount(${…} 등)는 톰캣 기본으로 본다.
pub fn server_values(server_xml: &str, module: &str) -> (String, i32) {
    let Some(doc) = parse(server_xml) else { return (String::new(), TOMCAT_MAX_PARAMS) };
    // 이클립스가 적는 Context 는 source="org.eclipse.jst.jee.server:<모듈>" 이다
    let ctx = doc.descendants().find(|c| {
        c.has_tag_name("Context") && (c.attribute("source").and_then(|s| s.rsplit(':').next()) == Some(module) || c.attribute("docBase") == Some(module))
    });
    let path = ctx.and_then(|c| c.attribute("path")).unwrap_or("").trim().trim_end_matches('/');
    let path = if path.is_empty() { String::new() } else if path.starts_with('/') { path.to_string() } else { format!("/{path}") };
    let max = http_connector(&doc).and_then(|c| c.attribute("maxParameterCount")).and_then(|v| v.trim().parse().ok()).filter(|&n: &i32| n != 0).unwrap_or(TOMCAT_MAX_PARAMS);
    (path, max)
}

impl ServerDiff {
    pub fn warnings(&self) -> Vec<String> {
        let mut w = Vec::new();
        if !self.connector.is_empty() {
            let shown = |v: &str| if v.is_empty() { "(없음)".to_string() } else { v.to_string() };
            let list: Vec<String> = self.connector.iter().map(|(a, t, o)| format!("{a} 이클립스 {} · 실행기 {}", shown(t), shown(o))).collect();
            w.push(format!("HTTP 연결 설정이 이클립스와 다릅니다 ({}) — 같게 하려면 {{base}}/conf/server.xml 의 HTTP Connector 에 이클립스 값을 적으세요", list.join(" / ")));
        }
        if !self.lost_links.is_empty() {
            w.push(format!(
                "DB 연결(JNDI) 링크 {} 가 가리키는 항목이 이클립스 server.xml 에 없어 옮길 수 없습니다 — {{base}}/conf/context.xml 에 Resource 로 직접 넣으세요",
                self.lost_links.join(" · ")
            ));
        }
        if !self.classpath.is_empty() {
            w.push(format!(
                "이클립스 실행 설정의 Classpath 탭에 더한 항목이 있습니다 ({}) — 실행기는 넣지 않습니다. jar 라면 pom.xml 의존성으로 넣거나 src/main/webapp/WEB-INF/lib 에 두세요",
                self.classpath.join(" · ")
            ));
        }
        if !self.env.is_empty() {
            w.push(format!(
                "이클립스 실행 설정에 환경 변수가 있습니다 ({}) — 실행기는 넘기지 않습니다. Windows 환경 변수로 정하거나 JVM 인자(-D…)로 옮기세요",
                self.env.join(" · ")
            ));
        }
        w
    }
}

/// 프로젝트 폴더의 이클립스 설정. ws 를 주면 프로젝트에 인코딩이 없을 때 작업 공간 기본도 본다.
pub fn project_settings(project: &Path, ws: Option<&Path>) -> ProjectSettings {
    let read = |rel: &str| fs::read_to_string(project.join(rel)).unwrap_or_default();
    let mut s = ProjectSettings::default();
    // m2e 는 프로젝트 전체가 아니라 소스 폴더에 적는다 (encoding//src/main/java=UTF-8) — 컴파일하는 것은 그 폴더이므로 그쪽이 먼저다
    let mut whole = String::new();
    for (k, v) in prefs(&read(".settings/org.eclipse.core.resources.prefs")) {
        if k == "encoding//src/main/java" {
            s.encoding = v;
        } else if k == "encoding/<project>" {
            whole = v;
        } else if k.starts_with("encoding//src/main/java/") {
            s.file_encodings.push(v);
        }
    }
    if s.encoding.is_empty() {
        s.encoding = whole;
    }
    if s.encoding.is_empty() {
        if let Some(ws) = ws {
            let def = prefs(&fs::read_to_string(ws.join(".metadata/.plugins/org.eclipse.core.runtime/.settings/org.eclipse.core.resources.prefs")).unwrap_or_default())
                .into_iter()
                .find(|(k, _)| k == "encoding")
                .map(|(_, v)| v);
            s.ws_encoding = Some(def.unwrap_or_default());
        }
    }
    for (k, v) in prefs(&read(".settings/org.eclipse.jdt.core.prefs")) {
        match k.as_str() {
            "org.eclipse.jdt.core.compiler.compliance" => s.compliance = v,
            "org.eclipse.jdt.core.compiler.codegen.methodParameters" => s.method_parameters = v == "generate",
            _ => {}
        }
    }
    if let Some(d) = parse(&read(".settings/org.eclipse.wst.common.component")) {
        for n in d.descendants() {
            if n.has_tag_name("wb-resource") {
                let src = n.attribute("source-path").unwrap_or("").trim_end_matches('/');
                // target/ 아래(m2e-wtp 의 pom 정보 · 어노테이션 처리 결과)와 .apt_generated 는 빌드가 만드는 것이라 옮길 파일이 아니다
                let src = src.trim_start_matches('/').to_string();
                let generated = src.starts_with("target/") || src.starts_with(".apt_generated");
                if !src.is_empty() && !generated && !["src/main/webapp", "src/main/resources", "src/main/java"].contains(&src.as_str()) && !s.deploy_extra.contains(&src) {
                    s.deploy_extra.push(src);
                }
            } else if n.has_tag_name("dependent-module") {
                // module:/resource/<프로젝트>/<프로젝트> 만 — module:/classpath/… 는 Maven jar 라 deps 로 들어온다
                if let Some(rest) = n.attribute("handle").and_then(|h| h.strip_prefix("module:/resource/")) {
                    let name = rest.split('/').next().unwrap_or(rest).to_string();
                    if !name.is_empty() && !s.dependent.contains(&name) {
                        s.dependent.push(name);
                    }
                }
            }
        }
    }
    s
}

impl ProjectSettings {
    /// 가져올 때 서버 설정에 넣을 소스 인코딩 — 프로젝트 → 작업 공간 기본. 어디에도 없거나 UTF-8 이면 빈 값
    pub fn source_encoding(&self) -> String {
        let e = if self.encoding.is_empty() { self.ws_encoding.as_deref().unwrap_or("") } else { &self.encoding };
        clean_encoding(e)
    }

    /// 프로젝트 폴더만으로 다시 만들 수 있는 경고 — 점검할 때마다 쓴다.
    /// encoding · parameters 는 그 서버의 설정이다 (가져올 때 이클립스 값으로 채우므로, 서버 설정을 손으로 바꿨을 때만 경고가 뜬다)
    pub fn warnings(&self, encoding: &str, parameters: bool) -> Vec<String> {
        let mut w = Vec::new();
        if !self.encoding.is_empty() && !same_encoding(&self.encoding, encoding) {
            w.push(format!(
                "이클립스 소스 인코딩은 {} 인데 실행기는 {} 로 컴파일합니다 — 소스의 한글 문자열이 깨집니다. 설정 › 서버 › 소스 인코딩을 맞추세요",
                self.encoding,
                shown_encoding(encoding)
            ));
        }
        let odd = self.file_encodings.iter().filter(|e| !same_encoding(e, encoding)).count();
        if odd > 0 {
            w.push(format!("파일 · 폴더 {odd}개에 다른 인코딩을 따로 정했습니다 — 실행기는 소스 폴더 전체를 {} 로 컴파일하므로 그 파일의 한글이 깨집니다", shown_encoding(encoding)));
        }
        if !self.compliance.is_empty() && self.compliance != "1.8" {
            w.push(format!("이클립스는 자바 {} 기준으로 컴파일합니다 — 실행기는 1.8 기준이라 결과가 다르거나 컴파일 오류가 날 수 있습니다", self.compliance));
        }
        if self.method_parameters && !parameters {
            w.push("이클립스는 매개변수 이름을 남기고 컴파일합니다(-parameters) — 이 서버는 남기지 않아 @Param 없는 MyBatis 매퍼 · 이름 없는 @RequestParam 에서 오류가 납니다. 설정 › 서버 › 매개변수 이름을 켜세요".into());
        }
        if !self.deploy_extra.is_empty() {
            w.push(format!(
                "이클립스는 {} 도 배포합니다 — 실행기는 src/main/webapp · src/main/resources · 컴파일한 클래스만 올립니다. 필요한 파일은 그 폴더로 옮기세요",
                self.deploy_extra.join(" · ")
            ));
        }
        if !self.dependent.is_empty() {
            w.push(format!(
                "다른 프로젝트({})를 함께 배포합니다 — 실행기는 그 프로젝트를 컴파일하지 않고 이클립스 배포본의 jar 만 씁니다. 그 프로젝트를 고치면 이클립스에서 다시 배포한 뒤 의존 jar 를 다시 채우세요",
                self.dependent.join(" · ")
            ));
        }
        w
    }

    /// 프로젝트에도 작업 공간에도 인코딩을 정하지 않았다 — 작업 공간을 알 때만 참일 수 있다
    pub fn encoding_unset(&self) -> bool {
        self.ws_encoding.as_deref() == Some("")
    }

    /// 인코딩을 어디에도 정하지 않은 경우 — 「시작하기」 목록에만 보인다 (가져온 뒤에는 서버 설정에서 고친다)
    pub fn ws_warning(&self) -> Option<String> {
        // 정하지 않았으면 이클립스 판에 따라 다르다 — 2022-06 판부터 UTF-8, 그 전에는 Windows 기본(한국어 Windows 는 MS949)
        self.encoding_unset().then(|| {
            "소스 인코딩을 프로젝트에도 작업 공간에도 정하지 않아 UTF-8 로 컴파일합니다 — 이클립스가 Windows 기본(MS949)으로 읽었다면 한글 문자열이 깨집니다. 그러면 가져온 뒤 설정 › 서버 › 소스 인코딩을 MS949 로 바꾸세요".into()
        })
    }
}

pub fn read_workspace(ws: &Path) -> Workspace {
    let wtp = ws.join(WTP);
    let servers_xml = fs::read_to_string(wtp.join("servers.xml")).unwrap_or_default();
    let tmp_xml = fs::read_to_string(wtp.join("tmp-data.xml")).unwrap_or_default();
    let tmp_doc = parse(&tmp_xml);
    let mut out = Vec::new();
    if let Some(doc) = parse(&servers_xml) {
        for s in doc.descendants().filter(|n| n.has_tag_name("server")) {
            let id = s.attribute("id").unwrap_or("").to_string();
            let name = s.attribute("name").unwrap_or(&id).to_string();
            // 모듈 「shop::org.eclipse.jst.jee.server:shop::jst.web::2.5」 의 맨 앞이 프로젝트 이름
            let module = s
                .descendants()
                .find(|n| n.has_tag_name("list") && n.attribute("key") == Some("modules"))
                .and_then(|n| n.attribute("value0"))
                .and_then(|v| v.split("::").next())
                .unwrap_or("")
                .to_string();
            if module.is_empty() {
                continue;
            }
            let project = project_dir(ws, &module).map(|d| p(&d)).unwrap_or_default();
            // 포트 — 주석으로 막은 Connector 는 roxmltree 가 읽지 않는다
            let conf = s.attribute("configuration-id").unwrap_or("").trim_start_matches('/');
            let sx = fs::read_to_string(ws.join(conf).join("server.xml")).unwrap_or_default();
            let (mut shutdown, mut http, mut ajp) = (0u16, 0u16, 0u16);
            if let Some(d) = parse(&sx) {
                shutdown = d.root_element().attribute("port").and_then(|x| x.parse().ok()).unwrap_or(0);
                for c in d.descendants().filter(|n| n.has_tag_name("Connector")) {
                    let port = c.attribute("port").and_then(|x| x.parse().ok()).unwrap_or(0);
                    let proto = c.attribute("protocol").unwrap_or("HTTP/1.1");
                    if proto.starts_with("AJP") {
                        if ajp == 0 { ajp = port; }
                    } else if http == 0 && c.attribute("SSLEnabled") != Some("true") && port != 8443 {
                        http = port;
                    }
                }
            }
            // 배포 폴더 tmpN
            let tmp = tmp_doc
                .as_ref()
                .and_then(|d| d.descendants().find(|n| n.attribute("key") == Some(id.as_str())).and_then(|n| n.attribute("path")).map(String::from))
                .unwrap_or_default();
            let lib = wtp.join(&tmp).join(s.attribute("deployDir").unwrap_or("wtpwebapps")).join(&module).join("WEB-INF/lib");
            let (lib_dir, jars) = if !tmp.is_empty() && lib.is_dir() && !project.is_empty() {
                (p(&lib), extra_jars(&lib, &project).len())
            } else {
                (String::new(), 0)
            };
            let launch = fs::read_to_string(ws.join(".metadata/.plugins/org.eclipse.debug.core/.launches").join(format!("{id}.launch"))).unwrap_or_default();
            let cx = fs::read_to_string(ws.join(conf).join("context.xml")).unwrap_or_default();
            let mx = if project.is_empty() { String::new() } else { fs::read_to_string(Path::new(&project).join("src/main/webapp/META-INF/context.xml")).unwrap_or_default() };
            let naming_full = naming_entries(&sx, &cx, &mx, &module);
            let ps = if project.is_empty() { ProjectSettings::default() } else { project_settings(Path::new(&project), Some(ws)) };
            let server_warnings = server_diff(&sx, &launch, &naming_full).warnings();
            let (context_path, max_parameter_count) = server_values(&sx, &module);
            // 인코딩 · -parameters 는 서버 설정으로 옮겨 오므로 그 값과 견준다 — 남는 것은 파일별 인코딩뿐이다
            let (encoding, parameters) = (ps.source_encoding(), ps.method_parameters);
            // 리소스 필터링은 가져온 뒤 「상태」 탭의 「리소스 필터링」 줄이 다시 센다 — 서버 설정에 남기지 않는다
            let filtering = if project.is_empty() { None } else { crate::filtering::scan(Path::new(&project)).and_then(|r| r.warning()) };
            let warnings = server_warnings.iter().cloned().chain(ps.ws_warning()).chain(ps.warnings(&encoding, parameters)).chain(filtering).collect();
            let (java, java_name) = server_java(ws, &launch, s.attribute("runtime-id").unwrap_or("")).unwrap_or_default();
            out.push(EServer {
                java,
                java_name,
                java_version: String::new(),
                encoding,
                encoding_unset: ps.encoding_unset(),
                parameters,
                warnings,
                server_warnings,
                naming: naming_full.iter().map(brief).collect(),
                naming_full,
                tomcat_home: runtime_home(ws, s.attribute("runtime-id").unwrap_or("")),
                jvm_args: user_jvm_args(&launch),
                id,
                name,
                project,
                http_port: http,
                ajp_port: ajp,
                shutdown_port: shutdown,
                context_path,
                max_parameter_count,
                lib_dir,
                jars,
            });
        }
    }
    Workspace { path: p(ws), servers: out }
}

/// 이클립스 배포본의 jar 를 deps 로 — deps.new 에 다 옮긴 뒤에만 바꾼다. 옮긴 수를 돌려준다.
pub fn copy_deps(lib_dir: &str, project: &str, deps: &Path) -> Result<usize, String> {
    let lib = Path::new(lib_dir);
    let names = extra_jars(lib, project);
    if names.is_empty() {
        return Err(format!("이클립스 배포본에 옮길 jar 가 없습니다: {lib_dir}"));
    }
    let fresh = deps.with_file_name(format!("{}.new", deps.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| "deps".into())));
    let _ = fs::remove_dir_all(&fresh);
    fs::create_dir_all(&fresh).map_err(|e| format!("폴더를 만들지 못했습니다 ({}): {e}", p(&fresh)))?;
    for n in &names {
        fs::copy(lib.join(n), fresh.join(n)).map_err(|e| format!("{n} 을 복사하지 못했습니다: {e}"))?;
    }
    let old = fresh.with_extension("old");
    let _ = fs::remove_dir_all(&old);
    if deps.exists() {
        fs::rename(deps, &old).map_err(|e| format!("쓰던 jar 폴더를 옮기지 못했습니다 (톰캣이 켜져 있으면 끄세요): {e}"))?;
    }
    if let Err(e) = fs::rename(&fresh, deps) {
        let _ = fs::rename(&old, deps);
        return Err(format!("jar 폴더를 바꾸지 못했습니다: {e}"));
    }
    let _ = fs::remove_dir_all(&old);
    Ok(names.len())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 실행_인자에서_이클립스가_넣은_것은_뺀다() {
        let launch = r#"<launchConfiguration type="x"><stringAttribute key="org.eclipse.jdt.launching.VM_ARGUMENTS" value="-Dcatalina.base=&quot;D:\w\tmp0&quot; -Dcatalina.home=&quot;C:\Program Files\Tomcat 8.5&quot; -Dwtp.deploy=&quot;D:\w\tmp0\wtpwebapps&quot; -Djava.endorsed.dirs=&quot;C:\Program Files\Tomcat 8.5\endorsed&quot; -Dspring.profiles.active=local -Xmx2g"/></launchConfiguration>"#;
        assert_eq!(user_jvm_args(launch), ["-Dspring.profiles.active=local", "-Xmx2g"]);
    }

    #[test]
    fn jndi_는_링크를_풀고_그_모듈_것만_모은다() {
        let server = r#"<Server port="8005">
  <GlobalNamingResources>
    <Resource name="UserDatabase" auth="Container" type="org.apache.catalina.UserDatabase"/>
    <Resource name="jdbc/g" auth="Container" type="javax.sql.DataSource" username="u" password="p&amp;1" url="jdbc:x"/>
  </GlobalNamingResources>
  <Service><Engine><Host>
    <Context docBase="other" path="/other" source="org.eclipse.jst.jee.server:other"><Resource name="jdbc/other" type="javax.sql.DataSource"/></Context>
    <Context docBase="shop" path="/shop" source="org.eclipse.jst.jee.server:shop">
      <ResourceLink name="jdbc/main" global="jdbc/g" type="javax.sql.DataSource"/>
      <ResourceLink name="jdbc/lost" global="jdbc/none" type="javax.sql.DataSource"/>
      <Environment name="mode" type="java.lang.String" value="local"/>
    </Context>
  </Host></Engine></Service>
</Server>"#;
        let context = r#"<Context><WatchedResource>WEB-INF/web.xml</WatchedResource><Resource name="jdbc/ctx" type="javax.sql.DataSource" password="c"/><Resource name="mode" type="java.lang.String"/></Context>"#;
        let meta = r#"<Context><Resource name="jdbc/meta" type="javax.sql.DataSource"/></Context>"#;
        let v = naming_entries(server, context, meta, "shop");
        let names: Vec<&str> = v.iter().map(|e| e.name.as_str()).collect();
        // 다른 모듈 Context · 링크되지 않은 UserDatabase 는 빠지고, 이름이 겹치면 server.xml 이 이긴다
        assert_eq!(names, ["jdbc/main", "jdbc/lost", "mode", "jdbc/meta", "jdbc/ctx"]);
        let main = &v[0];
        assert_eq!(main.kind, "Resource");
        assert!(main.attrs.contains(&("name".into(), "jdbc/main".into())) && main.attrs.contains(&("password".into(), "p&1".into())));
        assert!(main.movable() && !v[1].movable());
        assert_eq!(v[2].from, "server.xml");
        let b: Vec<NamingBrief> = v.iter().map(brief).collect();
        assert!(b[0].secret && !b[2].secret && b[1].missing && b[4].secret);
        assert_eq!(b[0].type_, "javax.sql.DataSource");
    }

    #[test]
    fn 작업_공간에서_서버를_읽는다() {
        let ws = std::env::temp_dir().join(format!("tomcatrun-eclipse-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&ws);
        let w = |rel: &str, body: &str| {
            let f = ws.join(rel);
            fs::create_dir_all(f.parent().unwrap()).unwrap();
            fs::write(f, body).unwrap();
        };
        w(".metadata/.plugins/org.eclipse.wst.server.core/servers.xml", r#"<servers><server configuration-id="/Servers/T-config" deployDir="wtpwebapps" id="T" name="앱" runtime-id="Apache Tomcat v8.5"><list key="modules" value0="shop::org.eclipse.jst.jee.server:shop::jst.web::2.5"/></server></servers>"#);
        w(".metadata/.plugins/org.eclipse.wst.server.core/tmp-data.xml", r#"<temp-directories><temp-directory key="T" path="tmp0"/></temp-directories>"#);
        w("Servers/T-config/server.xml", r#"<Server port="9005"><Service><!-- <Connector port="1" protocol="AJP/1.3"/> --><Connector port="8888" protocol="HTTP/1.1"/><Connector port="8443" SSLEnabled="true"/><Connector port="8009" protocol="AJP/1.3"/></Service></Server>"#);
        w("shop/src/main/webapp/WEB-INF/lib/own.jar", "");
        // m2e 가 적는 꼴 — 프로젝트 전체가 아니라 소스 폴더에 인코딩이 붙는다
        w("shop/.settings/org.eclipse.core.resources.prefs", "eclipse.preferences.version=1\nencoding//src/main/java=UTF-8\nencoding//src/test/java=UTF-8\n");
        w(".metadata/.plugins/org.eclipse.wst.server.core/tmp0/wtpwebapps/shop/WEB-INF/lib/own.jar", "");
        w(".metadata/.plugins/org.eclipse.wst.server.core/tmp0/wtpwebapps/shop/WEB-INF/lib/dep-1.0.jar", "x");
        assert!(is_workspace(&ws));
        let r = read_workspace(&ws);
        assert_eq!(r.servers.len(), 1);
        let s = &r.servers[0];
        assert_eq!((s.name.as_str(), s.http_port, s.ajp_port, s.shutdown_port, s.jars), ("앱", 8888, 8009, 9005, 1));
        assert!(s.project.ends_with("/shop"));
        assert!(s.naming.is_empty());
        // Connector · Context 에 값이 없으면 톰캣 기본 — 서버 설정으로 옮겨 오므로 경고하지 않는다
        assert_eq!((s.context_path.as_str(), s.max_parameter_count), ("", 10000));
        assert!(s.warnings.is_empty(), "{:?}", s.warnings);
        assert_eq!((s.encoding.as_str(), s.parameters, s.encoding_unset), ("", false, false));
        let deps = ws.join("out/deps");
        fs::create_dir_all(ws.join("out")).unwrap();
        assert_eq!(copy_deps(&s.lib_dir, &s.project, &deps).unwrap(), 1);
        assert!(deps.join("dep-1.0.jar").is_file() && !deps.join("own.jar").exists());
        let _ = fs::remove_dir_all(&ws);
    }

    #[test]
    fn 접속_경로와_파라미터_수_상한을_읽는다() {
        let x = |ctx: &str, conn: &str| {
            server_values(&format!(r#"<Server><Service><Connector port="8009" protocol="AJP/1.3" maxParameterCount="5"/><Connector port="8080" {conn}/><Engine><Host><Context docBase="other" path="/other"/>{ctx}</Host></Engine></Service></Server>"#), "shop")
        };
        assert_eq!(x(r#"<Context docBase="shop" path="/shop/" source="org.eclipse.jst.jee.server:shop"/>"#, r#"maxParameterCount="-1""#), ("/shop".into(), -1));
        // 「/」 · 빈 값은 루트 · AJP 의 값은 보지 않는다 · 숫자가 아니면 톰캣 기본
        assert_eq!(x(r#"<Context docBase="shop" path="/"/>"#, r#"maxParameterCount="${x}""#), (String::new(), 10000));
        assert_eq!(x("", ""), (String::new(), 10000));
        assert_eq!(x(r#"<Context docBase="shop" path="a/b"/>"#, r#"maxParameterCount="20000""#), ("/a/b".into(), 20000));
    }

    #[test]
    fn 서버가_쓰던_jre_를_찾는다() {
        let ws = std::env::temp_dir().join(format!("tomcatrun-eclipse-jre-{}", std::process::id()));
        let _ = fs::remove_dir_all(&ws);
        let w = |rel: &str, body: &str| {
            let f = ws.join(rel);
            fs::create_dir_all(f.parent().unwrap()).unwrap();
            fs::write(f, body).unwrap();
        };
        w("jdk 17/bin/java.exe", "");
        w("jdk8/bin/java.exe", "");
        w("자바11/bin/java.exe", "");
        // 이클립스가 적는 꼴 — XML 한 줄에 「\r\n」 · 「\:」 · 「\=」, 한글은 「\uXXXX」
        let home = |d: &str| p(&ws.join(d)).replace(':', "\\:");
        let vm_xml = format!(
            r#"org.eclipse.jdt.launching.PREF_VM_XML=<?xml version\="1.0" encoding\="UTF-8" standalone\="no"?>\r\n<vmSettings defaultVM\="57,org.eclipse.jdt.internal.debug.ui.launcher.StandardVMType1,1">\r\n<vmType id\="org.eclipse.jdt.internal.debug.ui.launcher.StandardVMType">\r\n<vm id\="1" name\="jdk8" path\="{}"/>\r\n<vm id\="2" name\="jdk-17" path\="{}"/>\r\n<vm id\="3" name\="java11" path\="{}"/>\r\n</vmType>\r\n</vmSettings>\r\n"#,
            home("jdk8"),
            home("jdk 17"),
            home("자바11").replace("자바", "\\uC790\\uBC14"),
        );
        w(".metadata/.plugins/org.eclipse.core.runtime/.settings/org.eclipse.jdt.launching.prefs", &vm_xml);
        w(
            ".metadata/.plugins/org.eclipse.core.runtime/.settings/org.eclipse.wst.server.core.prefs",
            r#"runtimes=<?xml version\="1.0" encoding\="UTF-8" standalone\="no"?>\r\n<runtimes>\r\n<runtime id\="R11" location\="C\:\\none" vm-install-id\="3"/>\r\n</runtimes>\r\n"#,
        );
        let launch = |vm: &str| format!(r#"<launchConfiguration type="x"><stringAttribute key="org.eclipse.jdt.launching.JRE_CONTAINER" value="org.eclipse.jdt.launching.JRE_CONTAINER/org.eclipse.jdt.internal.debug.ui.launcher.StandardVMType/{vm}"/></launchConfiguration>"#);
        let found = |launch: &str, rt: &str| server_java(&ws, launch, rt).map(|(j, n)| (j.ends_with("/bin/java.exe"), n));
        // launch 의 이름이 먼저 · 실행 환경 이름(JavaSE-17)이면 런타임 · 없으면 작업 공간 기본
        assert_eq!(found(&launch("jdk-17"), "R11"), Some((true, "jdk-17".into())));
        assert_eq!(found(&launch("JavaSE-17"), "R11"), Some((true, "java11".into())));
        assert_eq!(found("", "없음"), Some((true, "jdk8".into())));
        assert!(server_java(&ws, "", "R11").unwrap().0.contains("/자바11/"));
        assert_eq!(composite_last("57,org.eclipse.jdt.internal.debug.ui.launcher.StandardVMType13,1700000000000"), "1700000000000");
        let _ = fs::remove_dir_all(&ws);
    }

    #[test]
    fn prefs_는_이스케이프된_키를_나눈다() {
        let v = prefs("#주석\neclipse.preferences.version=1\nencoding//src/main/java/a\\=b.java=MS949\nx\\:y = 1\n");
        assert_eq!(v, [("eclipse.preferences.version".into(), "1".into()), ("encoding//src/main/java/a=b.java".into(), "MS949".into()), ("x:y".into(), "1".into())]);
    }

    #[test]
    fn 인코딩_이름을_견준다() {
        assert_eq!((clean_encoding(" utf8 "), clean_encoding(" MS949 ")), (String::new(), "MS949".to_string()));
        assert!(same_encoding("", "UTF-8") && same_encoding("MS949", "x-windows-949") && same_encoding("cp949", "MS949"));
        assert!(!same_encoding("MS949", "EUC-KR") && !same_encoding("", "MS949"));
        // 어디에도 정하지 않았을 때만 (작업 공간을 모르면 판단하지 않는다)
        let unset = ProjectSettings { ws_encoding: Some(String::new()), ..Default::default() };
        assert!(unset.encoding_unset() && unset.ws_warning().is_some() && unset.source_encoding().is_empty());
        assert!(!ProjectSettings::default().encoding_unset());
    }

    #[test]
    fn 이클립스와_다른_설정을_경고한다() {
        let ws = std::env::temp_dir().join(format!("tomcatrun-eclipse-diff-{}", std::process::id()));
        let _ = fs::remove_dir_all(&ws);
        let w = |rel: &str, body: &str| {
            let f = ws.join(rel);
            fs::create_dir_all(f.parent().unwrap()).unwrap();
            fs::write(f, body).unwrap();
        };
        w(".metadata/.plugins/org.eclipse.wst.server.core/servers.xml", r#"<servers><server configuration-id="/Servers/T-config" id="T" name="앱"><list key="modules" value0="shop::org.eclipse.jst.jee.server:shop::jst.web::2.5"/></server></servers>"#);
        w("Servers/T-config/server.xml", r#"<Server port="8005"><Service>
  <Connector port="8080" protocol="HTTP/1.1" maxParameterCount="1000" URIEncoding="EUC-KR"/>
  <Engine><Host><Context docBase="shop" path="/shop" source="org.eclipse.jst.jee.server:shop"><ResourceLink name="jdbc/lost" global="jdbc/none"/></Context></Host></Engine>
</Service></Server>"#);
        let cp = |entry: &str| format!(r#"<listEntry value="{}"/>"#, entry.replace('"', "&quot;").replace('<', "&lt;"));
        w(
            ".metadata/.plugins/org.eclipse.debug.core/.launches/T.launch",
            &format!(
                r#"<launchConfiguration type="x"><listAttribute key="org.eclipse.jdt.launching.CLASSPATH">{}{}{}</listAttribute><mapAttribute key="org.eclipse.debug.core.environmentVariables"><mapEntry key="APP_ENV" value="local"/></mapAttribute></launchConfiguration>"#,
                cp(r#"<runtimeClasspathEntry containerPath="org.eclipse.jdt.launching.JRE_CONTAINER" path="1" type="4"/>"#),
                cp(r#"<runtimeClasspathEntry externalArchive="C:\tomcat\bin\bootstrap.jar" path="3" type="2"/>"#),
                cp(r#"<runtimeClasspathEntry externalArchive="C:\lib\ojdbc8.jar" path="3" type="2"/>"#),
            ),
        );
        w(".metadata/.plugins/org.eclipse.core.runtime/.settings/org.eclipse.core.resources.prefs", "encoding=MS949\n");
        w("shop/src/main/webapp/index.jsp", "");
        w("shop/.settings/org.eclipse.core.resources.prefs", "encoding//src/main/java/A.java=EUC-KR\nencoding//src/main/resources=UTF-8\n");
        w("shop/.settings/org.eclipse.jdt.core.prefs", "org.eclipse.jdt.core.compiler.compliance=11\norg.eclipse.jdt.core.compiler.codegen.methodParameters=generate\n");
        w(
            "shop/.settings/org.eclipse.wst.common.component",
            r#"<project-modules><wb-module deploy-name="shop">
  <wb-resource deploy-path="/" source-path="/src/main/webapp" tag="defaultRootSource"/>
  <wb-resource deploy-path="/WEB-INF/classes" source-path="/src/main/java"/>
  <wb-resource deploy-path="/WEB-INF/classes" source-path="/src/main/resources"/>
  <wb-resource deploy-path="/" source-path="/target/m2e-wtp/web-resources"/>
  <wb-resource deploy-path="/WEB-INF/classes" source-path="/.apt_generated"/>
  <wb-resource deploy-path="/WEB-INF/classes" source-path="/target/generated-sources/annotations"/>
  <wb-resource deploy-path="/WEB-INF/classes" source-path="/src/main/config"/>
  <dependent-module archiveName="common.jar" deploy-path="/WEB-INF/lib" handle="module:/resource/common/common"><dependency-type>uses</dependency-type></dependent-module>
  <dependent-module deploy-path="/WEB-INF/lib" handle="module:/classpath/var/M2_REPO/x.jar"/>
</wb-module></project-modules>"#,
        );
        w("shop/pom.xml", r#"<project xmlns="http://maven.apache.org/POM/4.0.0"><build><resources><resource><directory>src/main/resources</directory><filtering> true </filtering></resource></resources></build></project>"#);
        w("shop/src/main/resources/app.properties", "version=${project.version}\nurl=${db.url}\n");
        let r = read_workspace(&ws);
        let s = &r.servers[0];
        let has = |v: &[String], x: &str| v.iter().any(|m| m.contains(x));
        // 작업 공간에서만 읽을 수 있는 것 — 가져올 때 남긴다
        for x in ["URIEncoding 이클립스 EUC-KR · 실행기 UTF-8", "jdbc/lost", "(ojdbc8.jar)", "(APP_ENV)"] {
            assert!(has(&s.server_warnings, x), "{x} 없음: {:?}", s.server_warnings);
        }
        // 인코딩 · -parameters 는 서버 설정으로 옮겨 온다 — 프로젝트에 없으면 작업 공간 기본
        assert_eq!((s.encoding.as_str(), s.parameters, s.encoding_unset), ("MS949", true, false));
        assert!(!has(&s.warnings, "소스 인코딩") && !has(&s.warnings, "-parameters"), "{:?}", s.warnings);
        // 접속 경로 · maxParameterCount 는 서버 설정으로 옮겨 오므로 경고하지 않는다
        assert_eq!((s.context_path.as_str(), s.max_parameter_count), ("/shop", 1000));
        assert!(!has(&s.server_warnings, "maxParameterCount") && !has(&s.server_warnings, "/shop") && !has(&s.server_warnings, "bootstrap"));
        // 프로젝트 폴더에서 다시 읽을 수 있는 것 — 저장하지 않는다
        // 서버 설정을 UTF-8 · 끔으로 둔 채라면 -parameters 를 경고하고, 가져온 값이면 경고하지 않는다
        let ps = project_settings(Path::new(&s.project), None);
        let pw = ps.warnings("", false);
        for x in ["파일 · 폴더 1개", "자바 11", "-parameters", "src/main/config 도", "다른 프로젝트(common)"] {
            assert!(has(&pw, x), "{x} 없음: {pw:?}");
            assert!(!has(&s.server_warnings, x));
        }
        assert!(!has(&ps.warnings("MS949", true), "-parameters"));
        // 파일별 인코딩은 서버 인코딩과 같으면 세지 않는다 (src/main/resources 는 컴파일하지 않으므로 보지 않는다)
        assert!(!has(&ps.warnings("EUC-KR", true), "파일 · 폴더"));
        assert!(!has(&pw, "m2e-wtp") && !has(&pw, "apt_generated") && !has(&pw, "generated-sources") && ps.ws_encoding.is_none());
        // 리소스 필터링은 「시작하기」 목록에만 — 가져온 뒤에는 상태 탭이 다시 센다
        assert!(has(&s.warnings, "자리표시자가 1곳 있습니다 (app.properties (project.version))") && !has(&s.server_warnings, "필터링"));
        assert_eq!(s.warnings.len(), s.server_warnings.len() + ps.warnings(&s.encoding, s.parameters).len() + 1);
        let _ = fs::remove_dir_all(&ws);
    }
}

#[cfg(test)]
mod real {
    // 실제 작업 공간을 읽어 보인다 (읽기만): TOMCATRUN_TEST_ECLIPSE=<작업 공간> cargo test -- --ignored --nocapture
    #[test]
    #[ignore]
    fn 실제_작업_공간() {
        let Ok(ws) = std::env::var("TOMCATRUN_TEST_ECLIPSE") else { return };
        let started = std::time::Instant::now();
        let found = super::find_workspaces(&[]);
        println!("찾은 작업 공간 {:?} ({}ms)", found, started.elapsed().as_millis());
        let r = super::read_workspace(std::path::Path::new(&ws));
        println!("{}", serde_json::to_string_pretty(&r).unwrap());
    }
}
