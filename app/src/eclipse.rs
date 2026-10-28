// 이클립스 설정 읽기 — 「시작하기」 에서 이클립스 작업 공간의 톰캣 서버를 그대로 가져온다.
//
// 이클립스(WTP)가 남기는 것 (모두 읽기만 한다)
//   <작업 공간>/.metadata/.plugins/org.eclipse.wst.server.core/servers.xml   서버 이름 · 모듈(프로젝트) · 설정 폴더 · 런타임
//   <작업 공간>/Servers/<설정 폴더>/server.xml                               종료 · HTTP · AJP 포트
//   <작업 공간>/.metadata/.plugins/org.eclipse.wst.server.core/tmp-data.xml  서버마다 배포 폴더(tmpN)
//   tmpN/wtpwebapps/<프로젝트>/WEB-INF/lib                                    이클립스가 배포한 의존 jar — Maven 없이 deps 를 채운다
//   <작업 공간>/.metadata/.plugins/org.eclipse.core.runtime/.settings/org.eclipse.wst.server.core.prefs   런타임(톰캣) 폴더
//   <작업 공간>/.metadata/.plugins/org.eclipse.debug.core/.launches/<서버 id>.launch   사용자가 더한 JVM 인자
//   <작업 공간>/<프로젝트> 또는 .projects/<프로젝트>/.location                  프로젝트 폴더
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
    pub lib_dir: String,      // 이클립스가 배포한 WEB-INF/lib (없으면 빈 값)
    pub jars: usize,          // 그중 프로젝트 WEB-INF/lib 에 없는 것 = deps 로 옮길 것
    pub jvm_args: Vec<String>,
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

fn runtime_home(ws: &Path, runtime_id: &str) -> String {
    let prefs = fs::read_to_string(ws.join(".metadata/.plugins/org.eclipse.core.runtime/.settings/org.eclipse.wst.server.core.prefs")).unwrap_or_default();
    // runtimes=<?xml …> 한 줄 — 「\=」 「\:」 「\r\n」 처럼 이스케이프되어 있다
    let Some(line) = prefs.lines().find(|l| l.starts_with("runtimes=")) else { return String::new() };
    let xml = line["runtimes=".len()..].replace("\\r\\n", "\n").replace("\\=", "=").replace("\\:", ":").replace("\\\\", "\\");
    let Some(doc) = parse(&xml) else { return String::new() };
    doc.descendants()
        .find(|n| n.has_tag_name("runtime") && n.attribute("id") == Some(runtime_id))
        .and_then(|n| n.attribute("location"))
        .map(|l| l.replace('\\', "/"))
        .filter(|l| Path::new(l).join("bin/bootstrap.jar").is_file())
        .unwrap_or_default()
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
            out.push(EServer {
                tomcat_home: runtime_home(ws, s.attribute("runtime-id").unwrap_or("")),
                jvm_args: user_jvm_args(&launch),
                id,
                name,
                project,
                http_port: http,
                ajp_port: ajp,
                shutdown_port: shutdown,
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
        w(".metadata/.plugins/org.eclipse.wst.server.core/tmp0/wtpwebapps/shop/WEB-INF/lib/own.jar", "");
        w(".metadata/.plugins/org.eclipse.wst.server.core/tmp0/wtpwebapps/shop/WEB-INF/lib/dep-1.0.jar", "x");
        assert!(is_workspace(&ws));
        let r = read_workspace(&ws);
        assert_eq!(r.servers.len(), 1);
        let s = &r.servers[0];
        assert_eq!((s.name.as_str(), s.http_port, s.ajp_port, s.shutdown_port, s.jars), ("앱", 8888, 8009, 9005, 1));
        assert!(s.project.ends_with("/shop"));
        let deps = ws.join("out/deps");
        fs::create_dir_all(ws.join("out")).unwrap();
        assert_eq!(copy_deps(&s.lib_dir, &s.project, &deps).unwrap(), 1);
        assert!(deps.join("dep-1.0.jar").is_file() && !deps.join("own.jar").exists());
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
