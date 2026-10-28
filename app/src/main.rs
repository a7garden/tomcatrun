// tomcatrun — IDE 없이 로컬 톰캣을 띄우고, 자바를 고치면 컴파일해 실행 중인 앱에 바로 넣는다.
// 설정 파일에 등록한 서버마다 따로 켜고 끄며, 함께 띄울 수 있다.
//
// 구성 (모두 실행기 폴더 아래 · 아래 경로는 기본값이고 서버마다 설정에서 바꾼다)
//   base/ · classes/ · deps/                 첫 서버의 톰캣 설정(CATALINA_BASE) · 컴파일한 클래스 · 의존 jar
//   servers/<id>/base · classes · deps       그 밖의 서버 (conf 는 base/conf 를 복사해 쓴다)
//   bin/                                     DCEVM JDK 8 · HotswapAgent · Maven (모든 서버가 함께 쓴다)
//   tomcatrun.json                           경로 · 포트 · 옵션 · 버전 관리 (「설정」 탭에서 고친다)
//
// 프로젝트 경로 · 포트 · 접속 주소 · 저장소 주소처럼 쓰는 곳마다 다른 값은 코드와 배포본에 넣지 않는다.
// 처음 뜨면 빈 서버 하나로 시작하고, 사용자가 「설정 › 서버」에서 채운다.
//
// 반영 경로
//   JSP · JS · CSS              소스 폴더를 그대로 읽으므로 저장하면 끝
//   매퍼 XML                    앱이 묶음 전체를 다시 해석한다 (2~3초) · 「매퍼 하나만 다시 읽기」를 켜면 HotswapAgent 가 그 파일만
//   자바                        저장 → 감시 → 상주 컴파일러(ECJ · 바뀐 파일만) → HotswapAgent 가 실행 중인 JVM 에 교체
//   Spring 설정 · web.xml 등    기동 때만 읽는다 → 「재시작해야 반영」 목록에 올린다
//
// 원본은 건드리지 않는다 — 프로젝트 소스 · IDE 설정 · SVN 작업복사본 · git 저장소에는 읽기만 한다.
// 개발에서만 바꾸는 앱 설정은 overlay/ 의 별도 파일로 덧붙이고(tomcatrun.DevTweaks), 쓰는 것은 모두 이 폴더 안이다.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod eclipse;
mod git;
mod svn;
mod svnweb;
mod update;

use std::collections::HashSet;
use std::fs::{self, File, OpenOptions};
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::os::windows::io::AsRawHandle;
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::Mutex;
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use notify::{EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager, WindowEvent};

const CREATE_NO_WINDOW: u32 = 0x0800_0000;
const STAMP_FILE: &str = ".tomcatrun-stamp";

/* ---------- 설정 ---------- */

/// 서버 하나 (프로젝트 하나 = 톰캣 하나)
#[derive(Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
struct ServerCfg {
    id: String,
    name: String,
    project: String,
    catalina_base: String,
    classes_dir: String,
    deps_dir: String,
    http_port: u16,
    ajp_port: u16,
    shutdown_port: u16,
    debug_port: u16,
    jvm_args: Vec<String>,
    links: Vec<Link>,
    vcs: VcsCfg,
}

/// 서버(프로젝트)의 버전 관리 — 「설정 › 버전 관리」에서 고친다. 실행기는 읽기만 하고, 커밋 · 업데이트는 명령을 복사해 준다.
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
struct VcsCfg {
    kind: String,         // auto(폴더를 보고 정한다) · svn · git · both · none
    root: String,         // 작업복사본 · 저장소 폴더. 비면 project
    commit_hint: String,  // 커밋 메시지 칸에 흐리게 보일 예시 (팀의 메시지 규칙)
    svn_update: String,   // 「업데이트 명령 복사」가 만드는 명령 — 앞에 cd 줄이 붙는다
    git_update: String,
}

impl Default for VcsCfg {
    fn default() -> Self {
        VcsCfg {
            kind: "auto".into(),
            root: String::new(),
            commit_hint: String::new(),
            svn_update: "svn update".into(),
            // 원격과 갈라졌으면 합치지 않고 멈춘다 — 무엇이 들어올지 화면에서 먼저 본 뒤 사용자가 정한다
            git_update: "git pull --ff-only".into(),
        }
    }
}

const VCS_KINDS: [&str; 5] = ["auto", "svn", "git", "both", "none"];

fn vcs_root(s: &ServerCfg) -> String {
    if s.vcs.root.trim().is_empty() { s.project.clone() } else { s.vcs.root.trim().replace('\\', "/") }
}

/// 이 서버에서 볼 수 있는 버전 관리 — 「auto」 면 폴더에 .svn/wc.db · .git 이 있는지로 정한다
fn vcs_kinds(s: &ServerCfg) -> Vec<&'static str> {
    let root = PathBuf::from(vcs_root(s));
    let has_svn = || root.join(".svn").join("wc.db").exists();
    let has_git = || git::is_repo(&root);
    match s.vcs.kind.as_str() {
        "svn" => vec!["svn"],
        "git" => vec!["git"],
        "both" => vec!["svn", "git"],
        "none" => vec![],
        _ => [("svn", has_svn()), ("git", has_git())].into_iter().filter(|x| x.1).map(|x| x.0).collect(),
    }
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
struct Config {
    tomcat_home: String,
    javac: String,
    java_plain: String,
    java_hot: String,
    hotswap_agent: String,
    maven: String,
    editor: String,
    hot_reload: bool,
    auto_compile: bool,
    debug: bool,
    fast_jvm: bool,       // 개발용 JVM 옵션 (C1 만 · 검증 생략)
    heap_mb: u32,         // 톰캣 힙 상한 (0 이면 상한 없음 = JVM 기본값)
    stagger_start: bool,  // 두 서버를 동시에 누르면 하나씩 차례로 띄운다
    skip_scan: bool,      // 클래스 전체 탐색 건너뛰기 (web.xml 사본에 metadata-complete · 빈 absolute-ordering)
    compiler: String,     // 상주 컴파일러 — "ecj" | "javac"
    // ↓ 개발 전용 앱 설정 (overlay 의 tomcatrun.DevTweaks · 프로젝트 소스는 그대로)
    small_pool: bool,     // DB 연결 풀 처음 연결 수 20 → 1
    no_schedule: bool,    // @Scheduled 예약 작업을 돌리지 않는다
    lazy_init: bool,      // 빈을 처음 쓸 때 만든다 (실험)
    mapper_hotswap: bool, // 바뀐 매퍼 하나만 다시 읽는다 (실험 · HotswapAgent MyBatis 플러그인)
    keep_session: bool,   // 재시작해도 로그인 유지 (톰캣 기본값과 같다)
    update_check: bool,   // 켤 때 · 6시간마다 GitHub 릴리스에서 새 판을 확인한다
    tld_scan: String,     // TLD 를 찾을 jar (쉼표로 · 비우면 모든 jar) — server.xml 의 JarScanFilter
    ignore: Vec<String>,  // 저장 감시에서 뺄 파일 · 폴더 · 모든 서버 공통 · 프로젝트 기준 (「폴더/」 · 「*.sql」 · 「src/**/tmp」 꼴 · ignored() 참고)
    servers: Vec<ServerCfg>,
}

// 기동을 늦추는 HotswapAgent 플러그인. 이 앱에 없는 기술이라 켜 둘 까닭이 없다.
// 플러그인마다 클래스를 읽을 때마다 이름을 대조하므로, 줄이면 클래스 적재가 그만큼 가벼워진다.
// MyBatis 플러그인은 「매퍼 하나만 다시 읽기」를 켤 때만 살린다 (끄면 앱의 RefreshableSqlSessionFactoryBean 과 겹친다).
// 남기는 것: JdkPlugin · Hotswapper · WatchResources · ClassInitPlugin · AnonymousClassPatch · Spring · Tomcat · Logback · Proxy · ELResolver
const DISABLED_PLUGINS: [&str; 25] = [
    "Hibernate", "Hibernate3JPA", "Hibernate3", "Jersey1", "Jersey2", "Jetty", "ZK", "Log4j2", "MyFaces",
    "Mojarra", "Omnifaces", "WildFlyELResolver", "OsgiEquinox", "Owb", "WebObjects", "Weld", "JBossModules",
    "ResteasyRegistry", "Deltaspike", "GlassFish", "Vaadin", "Wicket", "CxfJAXRS", "FreeMarker", "Undertow",
];

// 개발에서만 쓰는 JVM 옵션. 운영보다 최고 속도는 낮지만 기동이 빠르다.
//   TieredStopAtLevel=1 : C2 최적화 컴파일러를 쓰지 않는다 — 기동 중 컴파일 부담이 준다
//   -Xverify:none       : 클래스를 읽을 때 바이트코드 검증을 건너뛴다 (컴파일러가 만든 클래스라 해가 없다)
// 2026-10-27 실측 : 매퍼 해석(기동 때 두 번) 12.6초 → 6.0초.
// 2026-10-27 : -Xms1g 를 뺐다. 힙을 처음부터 1GB 잡아 두는 것이 가볍게 하려는 쪽과 어긋난다 (힙 상한 heap_mb 로 다룬다).
const FAST_JVM_ARGS: [&str; 2] = ["-XX:TieredStopAtLevel=1", "-Xverify:none"];

// 힙 상한을 둘 때 함께 주는 GC 옵션. SerialGC 는 GC 스레드를 따로 두지 않아 가장 가볍고(코어 4개 PC),
// 쓰고 남은 힙을 운영체제에 돌려준다 (MaxHeapFreeRatio). DCEVM 이 지원하는 GC 이기도 하다.
// 상한이 없으면 기본이 RAM 의 1/4 (≈ 8GB) 이라, 같은 앱이 IDE 톰캣에서는 3.6~3.9GB 를 잡고 있었다 (2026-10-27 실측).
const LEAN_GC_ARGS: [&str; 3] = ["-XX:+UseSerialGC", "-XX:MinHeapFreeRatio=15", "-XX:MaxHeapFreeRatio=40"];

// 상주 컴파일러 JVM. 기본 설정이면 전용 메모리 1,240MB 를 잡는데 이렇게 묶으면 548MB 이고 속도는 같다 (2026-10-27 실측).
// 961개 전체 컴파일도 512MB 안에서 된다.
const DAEMON_JVM_ARGS: [&str; 4] = ["-Xmx512m", "-XX:+UseSerialGC", "-XX:TieredStopAtLevel=1", "-Dfile.encoding=UTF-8"];

// 이만큼 안 쓰면 상주 컴파일러를 내린다 (약 300MB 를 돌려준다). 다시 띄우는 비용은 ECJ 첫 컴파일 1.1초.
const DAEMON_IDLE_MINUTES: u64 = 20;

#[derive(Clone, Serialize, Deserialize)]
struct Link {
    label: String,
    url: String,
}

fn home_dir() -> PathBuf {
    if let Ok(p) = std::env::var("TOMCATRUN_HOME") {
        return PathBuf::from(p);
    }
    // 실행 파일이 app/target/release 안에 있어도 위로 올라가며 실행기 폴더(base/conf/server.xml 이 있는 곳)를 찾는다
    if let Ok(exe) = std::env::current_exe() {
        let mut d = exe.parent().map(Path::to_path_buf);
        while let Some(dir) = d {
            if dir.join("base").join("conf").join("server.xml").exists() {
                return dir;
            }
            d = dir.parent().map(Path::to_path_buf);
        }
    }
    // 2026-10-28 : 고정된 개발 폴더를 마지막 기본값으로 두던 것을 exe 폴더로 바꿨다.
    // 그 폴더가 남아 있으면 설정 · 세션 파일 · 컴파일 결과가 조용히 그쪽으로 갔다.
    std::env::current_exe().ok().and_then(|e| e.parent().map(Path::to_path_buf)).unwrap_or_default()
}

fn p(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

fn link(label: &str, url: &str) -> Link {
    Link { label: label.into(), url: url.into() }
}

impl Default for Config {
    fn default() -> Self {
        let h = home_dir();
        // 함께 까는 DCEVM JDK 는 온전한 JDK 8 이다 — 핫리로드를 끄면 같은 java 를 DCEVM 없이 띄우고, javac · tools.jar 도 이것을 쓴다
        let jdk = h.join("bin/jdk8-dcevm");
        Config {
            tomcat_home: "C:/Program Files/Apache Software Foundation/Tomcat 8.5".into(),
            javac: p(&jdk.join("bin/javac.exe")),
            java_plain: p(&jdk.join("bin/java.exe")),
            java_hot: p(&h.join("bin/jdk8-dcevm/bin/java.exe")),
            hotswap_agent: p(&h.join("bin/hotswap-agent.jar")),
            maven: p(&h.join("bin/maven/bin/mvn.cmd")),
            editor: "code".into(),
            hot_reload: true,
            auto_compile: true,
            debug: false,
            fast_jvm: true,
            heap_mb: 1536,
            stagger_start: true,
            // 서블릿 애너테이션 · 초기화 클래스를 쓰는 앱이 깨지지 않게 기본은 끈다. 쓰지 않는 앱이면 켜서 기동을 줄인다
            skip_scan: false,
            compiler: "ecj".into(),
            small_pool: true,
            no_schedule: false,
            lazy_init: false,
            mapper_hotswap: false,
            keep_session: true,
            update_check: true,
            tld_scan: String::new(),
            ignore: vec![],
            // 처음에는 빈 서버 하나 — 프로젝트 폴더 · 포트 · 접속 주소는 사용자가 「설정 › 서버」에서 정한다
            servers: vec![new_server(&h, "app", "앱", 0)],
        }
    }
}

/// 서버 하나의 기본값. n 번째 서버는 포트를 n 만큼 올려 다른 서버와 겹치지 않게 한다.
/// 첫 서버는 실행기 폴더의 base · classes · deps 를, 그 밖의 서버는 servers/<id>/ 아래를 쓴다.
fn new_server(h: &Path, id: &str, name: &str, n: u16) -> ServerCfg {
    let dir = if n == 0 { h.to_path_buf() } else { h.join("servers").join(id) };
    let http = 8080 + n;
    ServerCfg {
        id: id.into(),
        name: name.into(),
        project: String::new(),
        catalina_base: p(&dir.join("base")),
        classes_dir: p(&dir.join("classes")),
        deps_dir: p(&dir.join("deps")),
        http_port: http,
        ajp_port: 8009 + n,
        shutdown_port: 8005 + n * 100,
        debug_port: 8000 + n,
        jvm_args: vec![],
        vcs: VcsCfg::default(),
        // 2026-10-28 : 「http://localhost:8080/」 대신 「/」. 「/」로 시작하는 주소는 화면이 열 때 그 서버의 HTTP 포트를 붙이므로
        // 포트를 바꿔도 열기 단추가 따라간다 (서버를 더할 때 포트를 올려도 링크를 다시 만들 필요가 없다)
        links: vec![link("HTTP", "/")],
    }
}

/// 이 폴더에서 src/main/webapp 이 있는 프로젝트 폴더를 찾는다 — 그 폴더 자신, 없으면 바로 아래 한 단계.
/// 저장소 맨 위(trunk 등)를 고르는 일이 흔해서 한 단계까지 내려가 본다. 더 깊이는 보지 않는다 (큰 폴더에서 오래 걸린다).
fn webapp_roots(dir: &Path) -> Vec<String> {
    let is_root = |d: &Path| d.join("src/main/webapp").is_dir();
    if is_root(dir) {
        return vec![p(dir)];
    }
    let mut v: Vec<String> = fs::read_dir(dir).into_iter().flatten().flatten()
        .map(|e| e.path())
        .filter(|d| d.is_dir() && is_root(d.as_path()))
        .map(|d| p(&d))
        .collect();
    v.sort();
    v
}

const CONFIG_FILE: &str = "tomcatrun.json";

fn config_path() -> PathBuf {
    home_dir().join(CONFIG_FILE)
}

fn parse_config(text: &str) -> Result<Config, String> {
    let mut c: Config = serde_json::from_str(text).map_err(|e| e.to_string())?;
    fill_paths(&mut c, &Config::default());
    Ok(c)
}

// 실행기 폴더 안을 가리키는 경로 칸 — 비어 있으면 실행기 폴더(home_dir)에서 계산하고, 저장할 때 기본값과 같으면 비워 쓴다.
// 상대 경로로 적으면 실행기 폴더 기준이고, 실행기 폴더 안의 경로는 상대 경로로 저장한다.
// 2026-10-28 : 설정 파일에 절대 경로가 박혀 있어 폴더 이름을 바꾸거나 설치본으로 옮기면
// 세션 파일 · 컴파일 결과 · 로그가 옛 폴더로 갔다.
fn path_fields(c: &mut Config) -> Vec<&mut String> {
    let mut v = vec![&mut c.javac, &mut c.java_plain, &mut c.java_hot, &mut c.hotswap_agent, &mut c.maven];
    for s in c.servers.iter_mut() {
        v.extend([&mut s.catalina_base, &mut s.classes_dir, &mut s.deps_dir]);
    }
    v
}

// 서버 폴더의 기본값은 순서와 id 로 정한다 — 첫 서버는 실행기 폴더의 base · classes · deps, 그 밖은 servers/<id>/ (new_server)
fn default_paths(c: &Config, def: &Config) -> Vec<String> {
    let mut v = vec![def.javac.clone(), def.java_plain.clone(), def.java_hot.clone(), def.hotswap_agent.clone(), def.maven.clone()];
    for (n, s) in c.servers.iter().enumerate() {
        let d = new_server(&home_dir(), &s.id, "", n as u16);
        v.extend([d.catalina_base, d.classes_dir, d.deps_dir]);
    }
    v
}

fn fill_paths(c: &mut Config, def: &Config) {
    let defs = default_paths(c, def);
    let home = home_dir();
    for (f, d) in path_fields(c).into_iter().zip(defs) {
        let t = f.trim().replace('\\', "/");
        *f = if t.is_empty() {
            d
        } else if Path::new(&t).is_relative() {
            p(&home.join(&t))
        } else {
            t
        };
    }
}

fn strip_paths(c: &Config, def: &Config) -> Config {
    let norm = |a: &str| a.trim().replace('\\', "/").trim_end_matches('/').to_lowercase();
    let home = norm(&p(&home_dir()));
    let mut out = c.clone();
    let defs = default_paths(&out, def);
    for (f, d) in path_fields(&mut out).into_iter().zip(defs) {
        let n = norm(f);
        if n == norm(&d) {
            f.clear();
        } else if !home.is_empty() && n.starts_with(&format!("{home}/")) {
            *f = f.trim().replace('\\', "/")[home.len() + 1..].to_string();
        }
    }
    out
}

fn load_config() -> (Config, Option<String>) {
    match fs::read_to_string(config_path()) {
        Ok(s) => match parse_config(&s) {
            Ok(c) => (c, None),
            // 고치다 틀린 설정 파일을 기본값으로 덮어쓰지 않는다
            Err(e) => (Config::default(), Some(format!("설정 파일을 읽지 못해 기본값으로 띄웠습니다: {e}"))),
        },
        Err(_) => {
            let c = Config::default();
            let _ = save_config(&c);
            (c, Some("처음 실행입니다. 「설정 › 서버」에서 서버 이름 · 프로젝트 폴더 · 포트 · 열기 단추를 정하세요. 이 PC 에 맞는 값은 실행기가 알아내지 않습니다.".into()))
        }
    }
}

fn save_config(c: &Config) -> std::io::Result<()> {
    fs::write(config_path(), serde_json::to_string_pretty(&strip_paths(c, &Config::default())).unwrap_or_default())
}

/* ---------- 상태 ---------- */

#[derive(Clone, Copy, Serialize, PartialEq)]
#[serde(rename_all = "lowercase")]
enum Status {
    Stopped,
    Starting,
    Running,
    Stopping,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct CompileResult {
    ok: bool,
    full: bool,
    files: usize,
    errors: usize,
    ms: u64,
    at_ms: u64,
}

/// 실행 중에 바뀌었지만 재시작해야 들어가는 것
#[derive(Clone, Serialize)]
struct Pending {
    path: String,
    why: &'static str,
}

/// 기동 단계 하나와 걸린 시간
#[derive(Clone, Serialize)]
struct Phase {
    label: &'static str,
    ms: u64,
}

// 톰캣 로그에서 기동 단계의 경계를 잡는 표지. 앞 단계가 끝난 시각 = 이 줄이 나온 시각.
// Spring 의 두 줄은 ServletContext.log 로 나와 톰캣 언어 설정과 무관하게 영어다.
const PHASE_MARKS: [(&str, &str); 5] = [
    ("VersionLoggerListener", "JVM · 에이전트"),
    ("Initialization processed in", "톰캣 초기화"),
    ("Initializing Spring root WebApplicationContext", "웹앱 준비 (jar 스캔 등)"),
    ("Initializing Spring FrameworkServlet", "Spring 루트 설정"),
    ("Server startup in", "Spring MVC · 마무리"),
];

/// 프로세스 메모리 (MB). 작업 집합 = 지금 실제 RAM 에 올라 있는 양 · 전용 = 그 프로세스가 확보한 양
#[derive(Clone, Copy, Serialize, Default, PartialEq)]
#[serde(rename_all = "camelCase")]
struct Mem {
    ws_mb: u32,
    private_mb: u32,
}

struct Inner {
    child: Option<Child>,
    status: Status,
    started_at_ms: Option<u64>,
    startup_ms: Option<u64>,
    spawn_ms: u64,                  // java 를 띄운 시각 (띄우기 전에는 0)
    marks: Vec<(usize, u64)>,       // 잡은 표지 (PHASE_MARKS 번호, 시각)
    phases: Vec<Phase>,             // 직전 기동의 단계별 시간
    mode: String,
    java: String, // 이번 기동에 쓴 java — 멈췄을 때 같은 JDK 의 jstack 으로 스레드를 뜬다
    waiting: bool,                  // 다른 서버가 다 뜨기를 기다리는 중 (차례로 띄우기)
    generation: u64,                // 시작할 때마다 하나씩 는다. 기다리던 옛 기동이 새 기동을 제 것으로 알고 이어 띄우지 않게 한다
    mapper_hotswap: bool,           // 이번 기동이 「매퍼 하나만 다시 읽기」로 떴는가
    mapper_wait: Option<(u64, String)>, // 매퍼를 저장하고 다시 읽었다는 로그를 기다리는 중 (시각, 파일)
    session_had: Option<u64>,       // 이번 기동 전에 남아 있던 세션 파일 크기 (없었거나 로그인 유지를 껐으면 None)
    session_err: Option<String>,    // 톰캣이 세션을 저장 · 되살리다 낸 오류의 첫 예외 줄 (빈 값이면 그 줄을 기다리는 중)
    mem: Mem,
    compiling: bool,
    last_compile: Option<CompileResult>,
    restart_needed: Vec<Pending>,
}

/// 서버 하나의 실행 상태
struct Rt {
    inner: Mutex<Inner>,
    compile_lock: Mutex<()>,
    compile_tx: Sender<bool>, // true = 전체 컴파일
    console: Mutex<Option<File>>,
    deps_busy: AtomicBool,
    live_mappers: Mutex<Vec<String>>,
    svn_stamp: AtomicU64, // 작업복사본 .svn/wc.db 의 수정 시각 — 바뀌면(커밋 · 업데이트) 버전 관리 화면을 다시 읽게 한다
    git_stamp: AtomicU64, // .git 의 index · HEAD · FETCH_HEAD 중 가장 늦은 수정 시각 — 같은 용도
}

struct Shared {
    cfg: Mutex<Config>,
    rts: Vec<Rt>,
    log_tx: Sender<LogLine>,
    change_tx: Sender<(usize, PathBuf)>,
    auto_compile: AtomicBool,
    watchers: Mutex<Vec<RecommendedWatcher>>,
    daemon: Mutex<Option<Daemon>>,
    daemon_alive: AtomicBool, // 화면 표시용. 컴파일 중에는 daemon 을 잠그고 있으므로 따로 둔다
    daemon_last_use: AtomicU64,
    daemon_kind: Mutex<String>, // "ECJ" | "javac" — 지금 떠 있는 상주 컴파일러
    daemon_mem: Mutex<Mem>,
    launcher_mem: Mutex<Mem>, // 실행기 + 화면 엔진(WebView2) 프로세스 합
    start_gate: Mutex<()>,    // 차례로 띄우기 — 한 서버가 「켜는 중」인 동안 다른 서버는 여기서 기다린다
    svn_creds: Mutex<std::collections::HashMap<String, (String, String)>>, // 이번 실행에서만 쓰는 SVN 계정 (호스트별 · 저장 안 함을 고른 경우)
    job: isize, // 실행기가 죽어도 톰캣이 남지 않게 묶는 작업 개체
    quitting: AtomicBool, // 닫기를 확인해 톰캣을 끄는 중 — 그 사이 닫기를 또 눌러도 다시 묻지 않는다
    update: Mutex<UpdateState>,
}

/// 업데이트 — 화면 위 알림 줄 · 설정 › 업데이트가 그린다
#[derive(Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
struct UpdateState {
    release: Option<update::Release>, // 이 판보다 새 판이 있으면
    installable: bool,                // 설치본이라 실행기 안에서 설치까지 할 수 있다
    checking: bool,
    checked_at_ms: Option<u64>,
    downloading: Option<u64>,         // 받은 바이트
    error: Option<String>,
}

/// 상주 컴파일러 (helper/TomcatrunCompiler) — 띄워 둔 채 저장마다 일을 준다
struct Daemon {
    child: Child,
    stdin: std::process::ChildStdin,
    stdout: BufReader<std::process::ChildStdout>,
    uses: u32,
}

// 같은 JVM 에서 컴파일을 오래 거듭하면 메모리가 쌓인다. 이만큼 쓰면 새로 띄운다.
const DAEMON_MAX_USES: u32 = 300;

// 앱 수명에 묶인 공유 상태 참조. app.state() 가 돌려주는 임시 값에 잠금을 걸면 그 값보다 잠금이 오래 살아 빌림 오류가 난다.
fn shared(app: &AppHandle) -> &Shared {
    app.state::<Shared>().inner()
}

fn rt(app: &AppHandle, i: usize) -> &Rt {
    // State 는 앱이 살아 있는 동안 그대로이므로 참조를 넘겨도 된다
    let s: tauri::State<'_, Shared> = app.state::<Shared>();
    let shared: &Shared = s.inner();
    &shared.rts[i]
}

fn cfgs(app: &AppHandle, i: usize) -> (Config, ServerCfg) {
    let c = app.state::<Shared>().cfg.lock().unwrap().clone();
    let s = c.servers[i].clone();
    (c, s)
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct Check {
    key: &'static str,
    label: &'static str,
    ok: bool,
    detail: String,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct ServerSnap {
    id: String,
    name: String,
    project: String,
    catalina_base: String,
    http_port: u16,
    ajp_port: u16,
    shutdown_port: u16,
    debug_port: u16,
    jvm_args: Vec<String>,
    links: Vec<Link>,
    status: Status,
    pid: Option<u32>,
    started_at_ms: Option<u64>,
    startup_ms: Option<u64>,
    mode: String,
    compiling: bool,
    last_compile: Option<CompileResult>,
    restart_needed: Vec<Pending>,
    deps_busy: bool,
    checks: Vec<Check>,
    phases: Vec<Phase>,
    waiting: bool,
    mem: Mem,
    vcs: VcsCfg,
    vcs_kinds: Vec<&'static str>, // 실제로 볼 수 있는 것 (svn · git)
    vcs_root: String,
}

/// 「설정 › 경로」 — 모든 서버가 함께 쓰는 것
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Paths {
    tomcat_home: String,
    javac: String,
    java_plain: String,
    editor: String,
    tld_scan: String,
}

/// 「설정 › 서버」 한 칸 — 서버 하나의 이름 · 프로젝트 · 포트 · 열기 단추 · JVM 인자
#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ServerEdit {
    name: String,
    project: String,
    http_port: u16,
    ajp_port: u16,
    shutdown_port: u16,
    debug_port: u16,
    links: Vec<Link>,
    jvm_args: Vec<String>,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct Snapshot {
    hot_reload: bool,
    auto_compile: bool,
    debug: bool,
    fast_jvm: bool,
    heap_mb: u32,
    stagger_start: bool,
    skip_scan: bool,
    compiler: String,
    small_pool: bool,
    no_schedule: bool,
    lazy_init: bool,
    mapper_hotswap: bool,
    keep_session: bool,
    ignore: Vec<String>,
    paths: Paths,
    update_check: bool,
    update: UpdateState,
    daemon: bool,
    daemon_kind: String,
    daemon_mem: Mem,
    launcher_mem: Mem,
    checks: Vec<Check>,
    servers: Vec<ServerSnap>,
    config_path: String,
    home: String,
    version: &'static str,
}

fn now_ms() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0)
}

fn mtime_ms(path: &Path) -> Option<u64> {
    fs::metadata(path).ok()?.modified().ok()?.duration_since(UNIX_EPOCH).ok().map(|d| d.as_millis() as u64)
}

fn read_stamp(dir: &str) -> u64 {
    fs::read_to_string(Path::new(dir).join(STAMP_FILE)).ok().and_then(|t| t.trim().parse().ok()).unwrap_or(0)
}

fn common_checks(c: &Config) -> Vec<Check> {
    let ex = |s: &str| Path::new(s).exists();
    let dcevm = Path::new(&c.java_hot)
        .parent()
        .and_then(Path::parent)
        .map(|jdk| jdk.join("jre/bin/dcevm/jvm.dll").exists())
        .unwrap_or(false);
    vec![
        Check { key: "tomcat", label: "톰캣 8.5", ok: ex(&format!("{}/bin/bootstrap.jar", c.tomcat_home)), detail: c.tomcat_home.clone() },
        Check { key: "jdk", label: "JDK 8 (javac)", ok: ex(&c.javac) && ex(&c.java_plain), detail: c.javac.clone() },
        Check { key: "dcevm", label: "DCEVM JDK 8", ok: ex(&c.java_hot) && dcevm, detail: c.java_hot.clone() },
        Check { key: "agent", label: "HotswapAgent", ok: ex(&c.hotswap_agent), detail: c.hotswap_agent.clone() },
        Check {
            key: "helper",
            label: "상주 컴파일러",
            ok: helper_dir().join("TomcatrunCompiler.class").exists(),
            detail: p(&helper_dir()),
        },
        Check {
            key: "ecj",
            label: "ECJ (이클립스 컴파일러)",
            ok: ecj_jar(c).is_some(),
            detail: ecj_jar(c).map(|j| p(&j)).unwrap_or_else(|| "톰캣 lib 에 없음 · javac 로 컴파일".into()),
        },
        Check {
            key: "overlay",
            label: "개발 전용 앱 설정",
            ok: overlay_dir().join("tomcatrun/DevTweaks.class").exists()
                && overlay_dir().join("spring/context-zz-tomcatrun.xml").exists()
                && overlay_dir().join("egovframework/springmvc/application-servlet.xml").exists(),
            detail: p(&overlay_dir()),
        },
    ]
}

// 톰캣이 JSP 컴파일에 쓰려고 lib 에 두는 이클립스 컴파일러 (8.5 는 ecj-4.6.3.jar)
fn ecj_jar(c: &Config) -> Option<PathBuf> {
    fs::read_dir(Path::new(&c.tomcat_home).join("lib")).ok()?.filter_map(Result::ok).map(|e| e.path()).find(|p| {
        let n = p.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
        n.starts_with("ecj-") && n.ends_with(".jar")
    })
}

fn helper_dir() -> PathBuf {
    home_dir().join("helper")
}

fn overlay_dir() -> PathBuf {
    home_dir().join("overlay")
}

fn server_checks(s: &ServerCfg) -> Vec<Check> {
    let jars = fs::read_dir(&s.deps_dir)
        .map(|r| r.filter_map(Result::ok).filter(|e| e.path().extension().map_or(false, |x| x == "jar")).count())
        .unwrap_or(0);
    // pom 을 고친 뒤 jar 를 다시 받지 않았으면 알린다
    let pom = mtime_ms(&Path::new(&s.project).join("pom.xml")).unwrap_or(0);
    let deps_fresh = read_stamp(&s.deps_dir) >= pom;
    let compiled = Path::new(&s.classes_dir).join(STAMP_FILE).exists();
    vec![
        Check { key: "project", label: "소스", ok: Path::new(&s.project).join("src/main/webapp").exists(), detail: s.project.clone() },
        Check {
            key: "deps",
            label: "의존 jar",
            ok: jars > 0 && deps_fresh,
            detail: if jars == 0 {
                "없음".into()
            } else if !deps_fresh {
                format!("{jars}개 · pom.xml 이 더 새롭습니다. 다시 받으세요")
            } else {
                format!("{jars}개")
            },
        },
        Check {
            key: "classes",
            label: "컴파일 결과",
            ok: compiled,
            detail: if compiled { s.classes_dir.clone() } else { "아직 없음 · 처음 시작할 때 전체 컴파일".into() },
        },
    ]
}

fn snapshot(app: &AppHandle) -> Snapshot {
    let c = app.state::<Shared>().cfg.lock().unwrap().clone();
    let servers = c
        .servers
        .iter()
        .enumerate()
        .map(|(n, s)| {
            let r = rt(app, n);
            let i = r.inner.lock().unwrap();
            ServerSnap {
                id: s.id.clone(),
                name: s.name.clone(),
                project: s.project.clone(),
                catalina_base: s.catalina_base.clone(),
                http_port: s.http_port,
                ajp_port: s.ajp_port,
                shutdown_port: s.shutdown_port,
                debug_port: s.debug_port,
                jvm_args: s.jvm_args.clone(),
                links: s.links.clone(),
                status: i.status,
                pid: i.child.as_ref().map(|c| c.id()),
                started_at_ms: i.started_at_ms,
                startup_ms: i.startup_ms,
                mode: i.mode.clone(),
                compiling: i.compiling,
                last_compile: i.last_compile.clone(),
                restart_needed: i.restart_needed.clone(),
                deps_busy: r.deps_busy.load(Ordering::Relaxed),
                checks: server_checks(s),
                phases: i.phases.clone(),
                waiting: i.waiting,
                mem: i.mem,
                vcs: s.vcs.clone(),
                vcs_kinds: vcs_kinds(s),
                vcs_root: vcs_root(s),
            }
        })
        .collect();
    let sh = shared(app);
    Snapshot {
        hot_reload: c.hot_reload,
        auto_compile: c.auto_compile,
        debug: c.debug,
        fast_jvm: c.fast_jvm,
        heap_mb: c.heap_mb,
        stagger_start: c.stagger_start,
        skip_scan: c.skip_scan,
        compiler: c.compiler.clone(),
        small_pool: c.small_pool,
        no_schedule: c.no_schedule,
        lazy_init: c.lazy_init,
        mapper_hotswap: c.mapper_hotswap,
        keep_session: c.keep_session,
        ignore: c.ignore.clone(),
        paths: Paths {
            tomcat_home: c.tomcat_home.clone(),
            javac: c.javac.clone(),
            java_plain: c.java_plain.clone(),
            editor: c.editor.clone(),
            tld_scan: c.tld_scan.clone(),
        },
        update_check: c.update_check,
        update: sh.update.lock().unwrap().clone(),
        daemon:sh.daemon_alive.load(Ordering::Relaxed),
        daemon_kind: sh.daemon_kind.lock().unwrap().clone(),
        daemon_mem: *sh.daemon_mem.lock().unwrap(),
        launcher_mem: *sh.launcher_mem.lock().unwrap(),
        checks: common_checks(&c),
        servers,
        config_path: p(&config_path()),
        home: p(&home_dir()),
        // 제목 줄에 보인다 — Cargo.toml 의 version 한 곳에서 온다
        version: env!("CARGO_PKG_VERSION"),
    }
}

fn emit_state(app: &AppHandle) {
    let _ = app.emit("state", snapshot(app));
}

fn set_status(app: &AppHandle, i: usize, st: Status) {
    rt(app, i).inner.lock().unwrap().status = st;
    emit_state(app);
}

fn add_pending(app: &AppHandle, i: usize, path: String, why: &'static str) {
    let mut inner = rt(app, i).inner.lock().unwrap();
    if !inner.restart_needed.iter().any(|x| x.path == path) {
        inner.restart_needed.push(Pending { path, why });
    }
}

/* ---------- 로그 ---------- */

#[derive(Clone, Serialize)]
struct LogLine {
    srv: Option<usize>, // 어느 서버 로그인지. 없으면 실행기 전체
    src: &'static str,  // tomcat · compile · deps · app
    kind: &'static str, // err · warn · ok · hot · live · ""
    text: String,
}

fn classify(t: &str) -> &'static str {
    if t.contains("HOTSWAP AGENT") {
        if t.contains("ERROR") || t.contains("Unable to redefine") {
            return "err";
        }
        if t.contains("RELOAD") || t.contains("Reloading") || t.contains("reloaded") {
            return "hot";
        }
    }
    if t.contains("Server startup in") || t.contains("BUILD SUCCESS") {
        return "ok";
    }
    if t.contains("ERROR") || t.contains("SEVERE") || t.contains("심각") || t.contains("Exception") || t.contains(": error:") || t.contains(": 오류:") {
        return "err";
    }
    if t.contains("WARN") || t.contains("경고") {
        return "warn";
    }
    ""
}

fn log(app: &AppHandle, srv: Option<usize>, src: &'static str, kind: &'static str, text: impl Into<String>) {
    let _ = app.state::<Shared>().log_tx.send(LogLine { srv, src, kind, text: text.into() });
}

// 기동 때 SQL 로그가 수천 줄 쏟아진다. 한 줄씩 보내면 화면이 멈추므로 80ms 마다 묶어 보낸다.
fn log_pump(app: AppHandle, rx: Receiver<LogLine>) {
    let mut buf: Vec<LogLine> = Vec::new();
    let mut last = Instant::now();
    loop {
        match rx.recv_timeout(Duration::from_millis(80)) {
            Ok(l) => buf.push(l),
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => return,
        }
        if !buf.is_empty() && (last.elapsed() >= Duration::from_millis(80) || buf.len() >= 400) {
            let _ = app.emit("logs", &buf);
            buf.clear();
            last = Instant::now();
        }
    }
}

// 톰캣 · 앱 로그는 MS949 로, HotswapAgent 일부는 UTF-8 로 나온다. 줄마다 UTF-8 이 아니면 MS949 로 읽는다.
fn decode(bytes: &[u8]) -> String {
    let s = match std::str::from_utf8(bytes) {
        Ok(s) => s.to_string(),
        Err(_) => encoding_rs::EUC_KR.decode(bytes).0.into_owned(),
    };
    s.trim_end_matches(['\r', '\n']).to_string()
}

fn pipe_reader(app: AppHandle, i: usize, src: &'static str, r: impl Read + Send + 'static) -> thread::JoinHandle<()> {
    thread::spawn(move || {
        for chunk in BufReader::new(r).split(b'\n') {
            let Ok(bytes) = chunk else { break };
            let text = decode(&bytes);
            if src == "tomcat" {
                let r = rt(&app, i);
                // 화면 로그는 8000줄만 들고 있으므로 전체는 파일로도 남긴다 (logs/console.log)
                if let Some(f) = r.console.lock().unwrap().as_mut() {
                    let _ = writeln!(f, "{text}");
                }
                if let Some(k) = PHASE_MARKS.iter().position(|(m, _)| text.contains(m)) {
                    record_mark(&app, i, k, &text);
                }
                if text.contains("HOTSWAP AGENT") && text.contains("Unable to redefine") {
                    add_pending(&app, i, "자바 클래스".into(), "구조가 바뀌어 실행 중인 앱에 넣지 못함");
                    emit_state(&app);
                }
                // 매퍼 하나만 다시 읽기 — HotswapAgent MyBatis 플러그인이 끝나면 남기는 줄
                if text.contains("MyBatis configuration refreshed") {
                    if let Some((_, file)) = r.inner.lock().unwrap().mapper_wait.take() {
                        log(&app, Some(i), "app", "hot", format!("매퍼를 다시 읽었습니다 · {file}"));
                    }
                }
                watch_session_line(&app, i, &text);
                // 힙 상한을 두었으므로 모자라면 여기서 알린다
                if text.contains("OutOfMemoryError") {
                    let (c, sv) = cfgs(&app, i);
                    let msg = if text.contains("Metaspace") || text.contains("PermGen") {
                        format!("{} 톰캣의 클래스 영역(Metaspace)이 모자랍니다. 자바를 많이 교체했다면 재시작하세요.", sv.name)
                    } else {
                        format!(
                            "{} 톰캣 메모리가 모자랍니다 (힙 상한 {}MB). 옵션의 「톰캣 힙 상한」을 올리고 재시작하세요.",
                            sv.name, c.heap_mb
                        )
                    };
                    log(&app, Some(i), "app", "err", msg.clone());
                    let _ = app.emit("error", msg);
                }
            }
            let kind = classify(&text);
            log(&app, Some(i), src, kind, text);
        }
    })
}

/* ---------- 재시작해도 로그인 유지 ---------- */

// 톰캣이 정상 종료 때 세션을 저장하는 곳 — conf/context.xml 의 <Manager pathname="SESSIONS.ser"> 는 work 폴더 기준이다.
// 두 server.xml 모두 엔진 Catalina · 호스트 localhost · 경로 "/"(작업 폴더 이름 ROOT) 하나다.
fn session_file(base: &str) -> PathBuf {
    Path::new(base).join("work/Catalina/localhost/ROOT/SESSIONS.ser")
}

fn kb(bytes: u64) -> String {
    format!("{} KB", bytes.div_ceil(1024))
}

// 스택 추적의 첫 줄 — 「java.io.InvalidClassException: …」 · 「Caused by: …」
fn exception_line(text: &str) -> Option<&str> {
    let t = text.trim_start();
    ((t.starts_with("java.") || t.starts_with("Caused by:")) && (t.contains("Exception") || t.contains("Error"))).then_some(t)
}

// 세션을 저장 · 되살리다 난 문제를 로그 줄에서 잡는다. 톰캣 메시지는 한국어로 번역되어 나오므로 문구가 아니라
// 로거 이름(StandardManager)과 예외 클래스로 잡는다. 둘 다 조용히 지나가면 「재시작하니 로그인이 풀렸다」로만 보인다.
fn watch_session_line(app: &AppHandle, i: usize, text: &str) {
    let r = rt(app, i);
    if text.contains("session.StandardManager") && (text.contains("심각") || text.contains("SEVERE")) {
        r.inner.lock().unwrap().session_err = Some(String::new());
        return;
    }
    let Some(cause) = exception_line(text) else { return };
    let (stopping, waiting) = {
        let inner = r.inner.lock().unwrap();
        // 끝난 뒤에 읽히는 마지막 줄도 있어 「꺼짐」도 종료 쪽으로 본다
        (matches!(inner.status, Status::Stopping | Status::Stopped), inner.session_err.as_deref() == Some(""))
    };
    // 저장할 때 직렬화하지 못한 값은 톰캣이 경고만 남기고 빼고 쓴다 — 다음 기동에 그 값이 없어 로그인이 풀린다
    let msg = if stopping && cause.contains("NotSerializableException") {
        let class = cause.rsplit(':').next().unwrap_or("").trim();
        format!("세션 값 하나를 저장하지 못했습니다 · 직렬화할 수 없는 클래스 {class} · 다음 기동에서 그 값이 빠져 로그인이 풀릴 수 있습니다.")
    } else if waiting {
        r.inner.lock().unwrap().session_err = Some(cause.to_string());
        if stopping {
            format!("로그인 세션을 저장하지 못했습니다: {cause}")
        } else {
            format!("저장한 로그인 세션을 되살리지 못했습니다: {cause}")
        }
    } else {
        return;
    };
    let (_, sv) = cfgs(app, i);
    let msg = format!("{} {msg}", sv.name);
    log(app, Some(i), "app", "err", msg.clone());
    let _ = app.emit("error", msg);
}

// 기동이 끝났을 때 — 띄우기 전에 세션 파일이 있었으면 되살렸는지 알린다. 톰캣은 읽고 나면 성공이든 실패든 파일을 지운다.
fn report_session_restore(app: &AppHandle, i: usize) {
    let (_, sv) = cfgs(app, i);
    let r = rt(app, i);
    let (had, err) = {
        let inner = r.inner.lock().unwrap();
        (inner.session_had, inner.session_err.clone())
    };
    let Some(size) = had else { return };
    let file = session_file(&sv.catalina_base);
    match err {
        Some(e) if !e.is_empty() => {} // 예외 줄을 잡을 때 이미 알렸다
        Some(_) => {
            let msg = format!("{} 저장한 로그인 세션을 되살리지 못했습니다. 톰캣 탭이나 logs/console.log 에서 원인을 보세요.", sv.name);
            log(app, Some(i), "app", "err", msg.clone());
            let _ = app.emit("error", msg);
        }
        None if file.exists() => log(
            app,
            Some(i),
            "app",
            "warn",
            format!("저장한 로그인 세션을 톰캣이 읽지 않았습니다 ({}). conf/context.xml 의 <Manager> 를 확인하세요.", p(&file)),
        ),
        None => log(app, Some(i), "app", "ok", format!("저장한 로그인 세션을 되살렸습니다 ({}).", kb(size))),
    }
}

// 정상 종료 뒤 — 세션 파일이 이번 종료 때 쓰였는지 본다. 세션이 하나도 없으면 톰캣은 파일을 만들지 않는다.
fn report_session_save(app: &AppHandle, i: usize, base: &str, since_ms: u64) {
    let file = session_file(base);
    let fresh = fs::metadata(&file).ok().filter(|_| mtime_ms(&file).unwrap_or(0) + 1000 >= since_ms);
    match fresh {
        Some(m) => log(app, Some(i), "app", "", format!("로그인 세션을 저장했습니다 ({}). 다음 기동 때 되살립니다.", kb(m.len()))),
        None => log(
            app,
            Some(i),
            "app",
            "warn",
            "세션 파일이 생기지 않았습니다. 로그인한 브라우저가 없었거나 저장에 실패했습니다 (톰캣 탭 참고). 다음 기동에서는 다시 로그인해야 합니다.",
        ),
    }
}

/* ---------- 기동 단계 측정 ---------- */

// 표지가 빠지면(로그 설정이 달라 그 줄이 안 나오면) 그 구간은 다음 표지의 구간에 합쳐진다
fn phases_from(spawn_ms: u64, marks: &[(usize, u64)]) -> Vec<Phase> {
    let mut sorted = marks.to_vec();
    sorted.sort();
    let mut prev = spawn_ms;
    sorted
        .into_iter()
        .map(|(k, t)| {
            let ph = Phase { label: PHASE_MARKS[k].1, ms: t.saturating_sub(prev) };
            prev = t;
            ph
        })
        .collect()
}

fn record_mark(app: &AppHandle, i: usize, k: usize, text: &str) {
    let r = rt(app, i);
    let now = now_ms();
    let last = k == PHASE_MARKS.len() - 1;
    let (phases, opts) = {
        let mut inner = r.inner.lock().unwrap();
        if inner.marks.iter().any(|(m, _)| *m == k) {
            return;
        }
        inner.marks.push((k, now));
        if !last {
            return;
        }
        inner.status = Status::Running;
        inner.startup_ms = text.find("Server startup in").and_then(|pos| text[pos..].chars().filter(|c| c.is_ascii_digit()).collect::<String>().parse().ok());
        inner.phases = phases_from(inner.spawn_ms, &inner.marks);
        (inner.phases.clone(), inner.mode.clone())
    };
    let total: u64 = phases.iter().map(|p| p.ms).sum();
    let detail: Vec<String> = phases.iter().map(|p| format!("{} {:.1}초", p.label, p.ms as f64 / 1000.0)).collect();
    log(app, Some(i), "app", "ok", format!("기동 {:.1}초 · {} · {}", total as f64 / 1000.0, detail.join(" · "), opts));
    // 옵션을 바꿔 가며 비교할 수 있게 기동마다 한 줄씩 쌓는다
    let (_, sv) = cfgs(app, i);
    let csv = Path::new(&sv.catalina_base).join("logs/startup-history.csv");
    let new_file = !csv.exists();
    if let Ok(mut f) = OpenOptions::new().create(true).append(true).open(&csv) {
        if new_file {
            let head: Vec<&str> = PHASE_MARKS.iter().map(|(_, l)| *l).collect();
            let _ = writeln!(f, "시각(ms),옵션,합계(ms),{}", head.join(","));
        }
        let mut cols = vec![String::new(); PHASE_MARKS.len()];
        let sorted = {
            let mut m = r.inner.lock().unwrap().marks.clone();
            m.sort();
            m
        };
        for (n, (k, _)) in sorted.iter().enumerate() {
            cols[*k] = phases.get(n).map(|p| p.ms.to_string()).unwrap_or_default();
        }
        let _ = writeln!(f, "{now},{},{total},{}", opts.replace(',', " "), cols.join(","));
    }
    report_session_restore(app, i);
    emit_state(app);
}

/* ---------- 톰캣 ---------- */

fn port_busy(port: u16) -> bool {
    let addr = SocketAddr::from(([127, 0, 0, 1], port));
    TcpStream::connect_timeout(&addr, Duration::from_millis(250)).is_ok()
}

// 포트를 잡고 있는 프로그램 이름. netstat 의 「상태」 열은 언어마다 달라 쓰지 않고, 상대 주소가 비어 있는 줄로 듣는 쪽을 가린다.
fn port_owner(port: u16) -> Option<String> {
    let out = Command::new("netstat").args(["-ano", "-p", "TCP"]).creation_flags(CREATE_NO_WINDOW).output().ok()?;
    let text = decode(&out.stdout);
    let suffix = format!(":{port}");
    let pid = text.lines().find_map(|l| {
        let c: Vec<&str> = l.split_whitespace().collect();
        (c.len() >= 5 && c[1].ends_with(&suffix) && (c[2] == "0.0.0.0:0" || c[2] == "[::]:0")).then(|| c[c.len() - 1].to_string())
    })?;
    let out = Command::new("tasklist")
        .args(["/FI", &format!("PID eq {pid}"), "/FO", "CSV", "/NH"])
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .ok()?;
    let name = decode(&out.stdout).split(',').next()?.trim().trim_matches('"').to_string();
    Some(if name.is_empty() { format!("PID {pid}") } else { format!("{name} (PID {pid})") })
}

fn assign_job(job: isize, child: &Child) {
    use windows_sys::Win32::System::JobObjects::AssignProcessToJobObject;
    if job != 0 {
        unsafe {
            AssignProcessToJobObject(job as _, child.as_raw_handle() as _);
        }
    }
}

fn make_job() -> isize {
    use windows_sys::Win32::System::JobObjects::{
        CreateJobObjectW, JobObjectExtendedLimitInformation, SetInformationJobObject, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
        JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
    };
    unsafe {
        let h = CreateJobObjectW(std::ptr::null(), std::ptr::null());
        if h.is_null() {
            return 0;
        }
        let mut info: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = std::mem::zeroed();
        info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        SetInformationJobObject(
            h,
            JobObjectExtendedLimitInformation,
            &info as *const _ as *const _,
            std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
        );
        h as isize
    }
}

/* ---------- 메모리 ---------- */

fn mem_of(handle: windows_sys::Win32::Foundation::HANDLE) -> Mem {
    use windows_sys::Win32::System::ProcessStatus::{K32GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS, PROCESS_MEMORY_COUNTERS_EX};
    unsafe {
        let mut c: PROCESS_MEMORY_COUNTERS_EX = std::mem::zeroed();
        c.cb = std::mem::size_of::<PROCESS_MEMORY_COUNTERS_EX>() as u32;
        if K32GetProcessMemoryInfo(handle, &mut c as *mut _ as *mut PROCESS_MEMORY_COUNTERS, c.cb) == 0 {
            return Mem::default();
        }
        Mem { ws_mb: (c.WorkingSetSize / 1_048_576) as u32, private_mb: (c.PrivateUsage / 1_048_576) as u32 }
    }
}

fn child_mem(child: &Child) -> Mem {
    mem_of(child.as_raw_handle() as _)
}

// 실행기 자신과 그 아래 화면 엔진(msedgewebview2) 프로세스의 합. 톰캣 · 상주 컴파일러는 따로 센다.
fn launcher_mem() -> Mem {
    use windows_sys::Win32::Foundation::{CloseHandle, INVALID_HANDLE_VALUE};
    use windows_sys::Win32::System::Diagnostics::ToolHelp::{
        CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W, TH32CS_SNAPPROCESS,
    };
    use windows_sys::Win32::System::Threading::{GetCurrentProcessId, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION};
    unsafe {
        let me = GetCurrentProcessId();
        let snap = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0);
        if snap == INVALID_HANDLE_VALUE {
            return Mem::default();
        }
        let mut all: Vec<(u32, u32, bool)> = Vec::new(); // (pid, 부모, 화면 엔진인가)
        let mut e: PROCESSENTRY32W = std::mem::zeroed();
        e.dwSize = std::mem::size_of::<PROCESSENTRY32W>() as u32;
        if Process32FirstW(snap, &mut e) != 0 {
            loop {
                let len = e.szExeFile.iter().position(|&c| c == 0).unwrap_or(e.szExeFile.len());
                let name = String::from_utf16_lossy(&e.szExeFile[..len]);
                all.push((e.th32ProcessID, e.th32ParentProcessID, name.eq_ignore_ascii_case("msedgewebview2.exe")));
                if Process32NextW(snap, &mut e) == 0 {
                    break;
                }
            }
        }
        CloseHandle(snap);
        let mut set = vec![me];
        let mut k = 0;
        while k < set.len() {
            let parent = set[k];
            for (pid, pp, webview) in &all {
                if *pp == parent && *webview && !set.contains(pid) {
                    set.push(*pid);
                }
            }
            k += 1;
        }
        let mut sum = Mem::default();
        for pid in set {
            let h = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
            if h.is_null() {
                continue;
            }
            let m = mem_of(h);
            CloseHandle(h);
            sum.ws_mb += m.ws_mb;
            sum.private_mb += m.private_mb;
        }
        sum
    }
}

#[derive(Clone, Serialize)]
struct MemSnap {
    servers: Vec<Mem>,
    daemon: Mem,
    launcher: Mem,
}

// 3초마다 메모리를 재서 화면에 보낸다. 같은 김에 매퍼 다시 읽기 확인 · 쉬는 상주 컴파일러 내리기도 한다.
fn monitor(app: AppHandle) {
    loop {
        thread::sleep(Duration::from_secs(3));
        let s = app.state::<Shared>();
        let mut servers = Vec::new();
        let mut changed = false;
        for i in 0..s.rts.len() {
            let r = rt(&app, i);
            let (m, overdue) = {
                let mut inner = r.inner.lock().unwrap();
                let m = inner.child.as_ref().map(child_mem).unwrap_or_default();
                inner.mem = m;
                // 매퍼를 저장했는데 8초 안에 「MyBatis configuration refreshed」 가 없으면 플러그인이 듣지 않은 것이다
                let overdue = match &inner.mapper_wait {
                    Some((t, _)) if now_ms().saturating_sub(*t) > 8_000 => inner.mapper_wait.take().map(|(_, f)| f),
                    _ => None,
                };
                (m, overdue)
            };
            if let Some(file) = overdue {
                add_pending(&app, i, file, "매퍼 하나만 다시 읽기가 반응하지 않음 · 그 옵션을 끄고 재시작하면 앱이 묶음 전체를 다시 읽는다");
                log(&app, Some(i), "app", "warn", "매퍼를 저장했지만 다시 읽었다는 기록이 없습니다. 「매퍼 하나만 다시 읽기」가 이 앱과 맞지 않는 것 같습니다.");
                changed = true;
            }
            servers.push(m);
        }
        let daemon = match s.daemon.try_lock() {
            Ok(g) => g.as_ref().map(|d| child_mem(&d.child)).unwrap_or_default(),
            Err(_) => *s.daemon_mem.lock().unwrap(), // 컴파일 중이면 직전 값
        };
        *s.daemon_mem.lock().unwrap() = daemon;
        let launcher = launcher_mem();
        *s.launcher_mem.lock().unwrap() = launcher;
        let _ = app.emit("mem", MemSnap { servers, daemon, launcher });
        reap_idle_daemon(&app);
        // 실행기 밖에서 커밋 · 업데이트 · fetch 하면 wc.db 나 .git 이 바뀐다 — 버전 관리 화면에 다시 읽으라고 알린다
        let roots: Vec<String> = s.cfg.lock().unwrap().servers.iter().map(vcs_root).collect();
        for (i, root) in roots.iter().enumerate() {
            let r = rt(&app, i);
            for (stamp, slot) in [(svn::db_stamp(Path::new(root)), &r.svn_stamp), (git::stamp(Path::new(root)), &r.git_stamp)] {
                if let Some(stamp) = stamp {
                    let prev = slot.swap(stamp, Ordering::Relaxed);
                    if prev != 0 && prev != stamp {
                        let _ = app.emit("vcs-dirty", i);
                    }
                }
            }
        }
        if changed {
            emit_state(&app);
        }
    }
}

/* ---------- 톰캣 설정 (server.xml 의 ${tomcatrun.*}) ---------- */

// server.xml 에는 경로 · 포트를 적지 않고 이 속성으로 받는다 — 설정 화면에서 고친 값이 다음 기동에 그대로 들어간다.
fn server_props(cfg: &Config, sv: &ServerCfg) -> Vec<String> {
    let proj = sv.project.trim().trim_end_matches(['/', '\\']).replace('\\', "/");
    let tld = cfg.tld_scan.trim();
    [
        ("shutdownPort", sv.shutdown_port.to_string()),
        ("httpPort", sv.http_port.to_string()),
        ("ajpPort", sv.ajp_port.to_string()),
        ("webapp", format!("{proj}/src/main/webapp")),
        ("resources", format!("{proj}/src/main/resources")),
        ("classes", sv.classes_dir.clone()),
        ("deps", sv.deps_dir.clone()),
        ("overlay", p(&overlay_dir())),
        ("tldScanAll", tld.is_empty().to_string()),
        ("tldScan", tld.to_string()),
        ("pluggabilityScan", (!cfg.skip_scan).to_string()),
        ("sciFilter", if cfg.skip_scan { r"org\.apache\.tomcat\.websocket\.server\.WsSci".into() } else { String::new() }),
    ]
    .into_iter()
    .map(|(k, v)| format!("-Dtomcatrun.{k}={v}"))
    .collect()
}

// 서버의 CATALINA_BASE 에 conf 가 없으면 실행기 폴더의 base/conf 를 복사한다 (서버를 새로 더했을 때).
// 이미 있으면 건드리지 않는다 — 사용자가 그 서버의 conf 만 고쳐 쓸 수 있게.
fn ensure_base(sv: &ServerCfg) -> Result<(), String> {
    let base = Path::new(&sv.catalina_base);
    let conf = base.join("conf");
    if !conf.join("server.xml").exists() {
        let tpl = home_dir().join("base/conf");
        if !tpl.join("server.xml").exists() {
            return Err(format!("톰캣 설정 원본이 없습니다: {}/server.xml", p(&tpl)));
        }
        fs::create_dir_all(&conf).map_err(|e| format!("톰캣 설정 폴더를 만들지 못했습니다 ({}): {e}", p(&conf)))?;
        for e in fs::read_dir(&tpl).map_err(|e| e.to_string())?.filter_map(Result::ok) {
            let name = e.file_name();
            // web-dev.xml 은 기동마다 다시 만든다
            if e.path().is_file() && name != "web-dev.xml" {
                fs::copy(e.path(), conf.join(&name)).map_err(|e| format!("톰캣 설정을 복사하지 못했습니다: {e}"))?;
            }
        }
    }
    for d in ["logs", "temp", "work", "webapps"] {
        let _ = fs::create_dir_all(base.join(d));
    }
    Ok(())
}

/* ---------- 클래스 전체 탐색 건너뛰기 ---------- */

fn find_bytes(hay: &[u8], needle: &[u8]) -> Option<usize> {
    hay.windows(needle.len()).position(|w| w == needle)
}

// 원본 web.xml 의 사본을 만든다 (server.xml 의 altDDName 이 이 사본을 가리킨다). 원본은 읽기만 한다.
// skip 이면 두 군데만 더한다 — 인코딩 · 줄바꿈을 건드리지 않도록 바이트 단위로 끼운다.
//   <web-app … metadata-complete="true">   애너테이션을 찾느라 클래스를 모두 훑지 않는다
//   <absolute-ordering/>                    앱 jar 의 초기화 클래스(Spring 의 WebApplicationInitializer 탐색)를 읽지 않는다
// 2026-10-27 실측 : 서블릿 3.0 애너테이션을 쓰지 않는 앱에서 이 탐색이 17.0초였고 찾은 것은 없었다.
fn dev_web_xml(src: &[u8], skip: bool) -> Option<Vec<u8>> {
    if !skip {
        return Some(src.to_vec());
    }
    let start = find_bytes(src, b"<web-app")?;
    let close = start + find_bytes(&src[start..], b">")?;
    let nl: &[u8] = if find_bytes(src, b"\r\n").is_some() { b"\r\n" } else { b"\n" };
    let mut out = Vec::with_capacity(src.len() + 200);
    out.extend_from_slice(&src[..close]);
    if find_bytes(&src[start..close], b"metadata-complete").is_none() {
        out.extend_from_slice(b" metadata-complete=\"true\"");
    }
    out.push(b'>');
    if find_bytes(src, b"absolute-ordering").is_none() {
        out.extend_from_slice(nl);
        out.extend_from_slice(b"\t<!-- tomcatrun: skip class scanning at startup (copy of the original web.xml) -->");
        out.extend_from_slice(nl);
        out.extend_from_slice(b"\t<absolute-ordering/>");
    }
    out.extend_from_slice(&src[close + 1..]);
    Some(out)
}

fn write_dev_web_xml(sv: &ServerCfg, skip: bool) -> Result<(), String> {
    let src_path = Path::new(&sv.project).join("src/main/webapp/WEB-INF/web.xml");
    let src = fs::read(&src_path).map_err(|e| format!("web.xml 을 읽지 못했습니다 ({}): {e}", p(&src_path)))?;
    let out = dev_web_xml(&src, skip).ok_or_else(|| format!("web.xml 에서 <web-app> 을 찾지 못했습니다: {}", p(&src_path)))?;
    let dst = Path::new(&sv.catalina_base).join("conf/web-dev.xml");
    fs::write(&dst, out).map_err(|e| format!("web.xml 사본을 쓰지 못했습니다 ({}): {e}", p(&dst)))
}

fn open_console_log(base: &str) -> Option<File> {
    let dir = Path::new(base).join("logs");
    let _ = fs::create_dir_all(&dir);
    let cur = dir.join("console.log");
    let _ = fs::rename(&cur, dir.join("console.prev.log"));
    OpenOptions::new().create(true).write(true).truncate(true).open(cur).ok()
}

fn start_server(app: &AppHandle, i: usize) -> Result<(), String> {
    let (cfg, sv) = cfgs(app, i);
    let r = rt(app, i);
    let name = sv.name.clone();
    if r.inner.lock().unwrap().status != Status::Stopped {
        return Err(format!("{name} 톰캣이 이미 켜져 있거나 켜는 중입니다."));
    }
    if r.deps_busy.load(Ordering::Relaxed) {
        return Err(format!("{name} 의존 jar 를 받는 중입니다. 끝난 뒤에 시작하세요."));
    }
    if !Path::new(&format!("{}/bin/bootstrap.jar", cfg.tomcat_home)).exists() {
        return Err(format!("톰캣을 찾지 못했습니다: {}", cfg.tomcat_home));
    }
    if sv.project.trim().is_empty() {
        return Err(format!("{name} 프로젝트 폴더를 정하지 않았습니다. 「설정 › 서버」에서 정하세요."));
    }
    if !Path::new(&sv.project).join("src/main/webapp").is_dir() {
        return Err(format!("{name} 프로젝트에 src/main/webapp 이 없습니다: {}", sv.project));
    }
    ensure_base(&sv).map_err(|e| format!("{name} {e}"))?;
    let java = if cfg.hot_reload { cfg.java_hot.clone() } else { cfg.java_plain.clone() };
    if !Path::new(&java).exists() {
        return Err(format!("java 를 찾지 못했습니다: {java}"));
    }
    if cfg.hot_reload && !Path::new(&cfg.hotswap_agent).exists() {
        return Err(format!("HotswapAgent 를 찾지 못했습니다: {}", cfg.hotswap_agent));
    }
    let mut ports = vec![("HTTP", sv.http_port), ("AJP", sv.ajp_port), ("종료", sv.shutdown_port)];
    if cfg.debug {
        ports.push(("디버거", sv.debug_port));
    }
    for (what, port) in ports {
        if port_busy(port) {
            let who = port_owner(port).map(|w| format!("{w} 가 ")).unwrap_or_default();
            return Err(format!("{name} {what} 포트 {port} 를 {who}쓰고 있습니다. IDE 에서 띄운 톰캣이면 거기서 먼저 끄세요."));
        }
    }
    *r.live_mappers.lock().unwrap() = live_mappers(&sv.project);
    // 매퍼 하나만 다시 읽기는 HotswapAgent MyBatis 플러그인이 하므로 핫리로드일 때만 켠다.
    // 핫리로드가 아닌데 앱의 전체 재해석까지 끄면 매퍼를 고쳐도 아무도 다시 읽지 않는다.
    let mapper_hotswap = cfg.mapper_hotswap && cfg.hot_reload;
    let generation = {
        let mut inner = r.inner.lock().unwrap();
        inner.generation += 1;
        inner.status = Status::Starting;
        inner.started_at_ms = Some(now_ms());
        inner.startup_ms = None;
        inner.spawn_ms = 0;
        inner.marks.clear();
        inner.restart_needed.clear();
        inner.mapper_hotswap = mapper_hotswap;
        inner.mapper_wait = None;
        let heap = format!("힙 {}MB", cfg.heap_mb);
        let mut mode = vec![if cfg.hot_reload { "핫리로드" } else { "일반" }];
        if cfg.fast_jvm {
            mode.push("빠른 JVM");
        }
        if cfg.heap_mb > 0 {
            mode.push(&heap);
        }
        if cfg.skip_scan {
            mode.push("탐색 건너뜀");
        }
        if cfg.small_pool {
            mode.push("DB 풀 1");
        }
        if cfg.no_schedule {
            mode.push("예약 작업 멈춤");
        }
        if cfg.lazy_init {
            mode.push("지연 초기화");
        }
        if mapper_hotswap {
            mode.push("매퍼 하나씩");
        }
        inner.mode = mode.join(" · ");
        inner.java = java.clone();
        inner.generation
    };
    emit_state(app);

    let app = app.clone();
    thread::spawn(move || {
        let app = &app;
        let r = rt(app, i);
        // 차례로 띄우기 — 코어가 4개뿐이라 두 서버가 동시에 뜨면 서로 느려진다. 먼저 누른 쪽이 다 뜰 때까지 기다린다.
        let gate = if cfg.stagger_start {
            let sh = shared(app);
            Some(match sh.start_gate.try_lock() {
                Ok(g) => g,
                Err(_) => {
                    r.inner.lock().unwrap().waiting = true;
                    emit_state(app);
                    log(app, Some(i), "app", "", "다른 서버가 다 뜰 때까지 기다립니다 (차례로 띄우기).");
                    let g = sh.start_gate.lock().unwrap_or_else(|e| e.into_inner());
                    r.inner.lock().unwrap().waiting = false;
                    g
                }
            })
        } else {
            None
        };
        // 기다리는 사이에 중지를 눌렀으면 그만둔다 (중지 뒤 다시 시작했다면 그 기동은 새 스레드가 맡는다)
        {
            let inner = r.inner.lock().unwrap();
            if inner.status != Status::Starting || inner.generation != generation {
                drop(inner);
                emit_state(app);
                return;
            }
        }
        if !Path::new(&sv.classes_dir).join(STAMP_FILE).exists() {
            log(&app, Some(i), "app", "", "컴파일한 클래스가 없어 전체 컴파일부터 합니다.");
            if !run_compile(app, i, true).map_or(false, |res| res.ok) {
                log(app, Some(i), "app", "err", "컴파일에 실패해 톰캣을 띄우지 않았습니다. 빌드 탭을 보세요.");
                set_status(app, i, Status::Stopped);
                return;
            }
        }
        let base = &sv.catalina_base;
        let home = &cfg.tomcat_home;
        let mut args: Vec<String> = Vec::new();
        if cfg.hot_reload {
            args.push("-XXaltjvm=dcevm".into());
            // autoHotswap 을 인자로 주면 앱 쪽에 hotswap-agent.properties 가 없어도 클래스 파일 감시가 켜진다 (HotswapperPlugin.init).
            // disablePlugin 은 여러 번 줄 수 있다 (HotswapAgent 가 하나씩 모은다).
            let mut disabled: Vec<String> = DISABLED_PLUGINS.iter().map(|n| format!("disablePlugin={n}")).collect();
            if !mapper_hotswap {
                disabled.push("disablePlugin=MyBatis".into());
            }
            args.push(format!("-javaagent:{}=autoHotswap=true,{}", cfg.hotswap_agent, disabled.join(",")));
        }
        if cfg.debug {
            args.push(format!("-agentlib:jdwp=transport=dt_socket,server=y,suspend=n,address={}", sv.debug_port));
        }
        if cfg.fast_jvm {
            args.extend(FAST_JVM_ARGS.iter().map(|s| s.to_string()));
        }
        if cfg.heap_mb > 0 {
            args.push(format!("-Xmx{}m", cfg.heap_mb));
            args.extend(LEAN_GC_ARGS.iter().map(|s| s.to_string()));
        }
        // 개발 전용 앱 설정 — overlay 의 tomcatrun.DevTweaks 가 읽는다. 속성이 없으면 그 항목은 손대지 않는다.
        if cfg.small_pool {
            args.push("-Dtomcatrun.pool.initialSize=1".into());
        }
        if cfg.no_schedule {
            args.push("-Dtomcatrun.noSchedule=true".into());
        }
        if cfg.lazy_init {
            args.push("-Dtomcatrun.lazy=true".into());
        }
        if mapper_hotswap {
            args.push("-Dtomcatrun.mapperHotswap=true".into());
        }
        // conf/context.xml 의 <Manager pathname="${tomcatrun.sessionFile}"> — 빈 값이면 세션을 저장하지 않는다
        args.push(format!("-Dtomcatrun.sessionFile={}", if cfg.keep_session { "SESSIONS.ser" } else { "" }));
        args.extend(server_props(&cfg, &sv));
        args.extend(sv.jvm_args.iter().cloned());
        args.extend([
            format!("-Dcatalina.base={base}"),
            format!("-Dcatalina.home={home}"),
            format!("-Djava.io.tmpdir={base}/temp"),
            format!("-Djava.util.logging.config.file={base}/conf/logging.properties"),
            "-Djava.util.logging.manager=org.apache.juli.ClassLoaderLogManager".into(),
            format!("-Djava.endorsed.dirs={home}/endorsed"),
            "-classpath".into(),
            format!("{home}/bin/bootstrap.jar;{home}/bin/tomcat-juli.jar"),
            "org.apache.catalina.startup.Bootstrap".into(),
            "start".into(),
        ]);
        let _ = fs::create_dir_all(Path::new(base).join("temp"));
        // server.xml 의 altDDName 이 가리키는 web.xml 사본. 원본이 바뀌었을 수 있으니 기동마다 다시 만든다.
        if let Err(e) = write_dev_web_xml(&sv, cfg.skip_scan) {
            log(app, Some(i), "app", "err", e);
            set_status(app, i, Status::Stopped);
            return;
        }
        {
            // 되살릴 세션이 있는지 띄우기 직전에 본다 — 기동이 끝나면 되살렸는지 알린다 (report_session_restore)
            let mut inner = r.inner.lock().unwrap();
            inner.session_had = if cfg.keep_session { fs::metadata(session_file(base)).ok().map(|m| m.len()) } else { None };
            inner.session_err = None;
        }
        *r.console.lock().unwrap() = open_console_log(base);
        log(app, Some(i), "app", "", format!("기동: {java} {}", args.join(" ")));
        r.inner.lock().unwrap().spawn_ms = now_ms();
        let spawned = Command::new(&java)
            .args(&args)
            .current_dir(base)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .creation_flags(CREATE_NO_WINDOW)
            .spawn();
        let mut child = match spawned {
            Ok(c) => c,
            Err(e) => {
                log(app, Some(i), "app", "err", format!("java 를 실행하지 못했습니다: {e}"));
                set_status(app, i, Status::Stopped);
                return;
            }
        };
        assign_job(app.state::<Shared>().job, &child);
        if let Some(o) = child.stdout.take() {
            pipe_reader(app.clone(), i, "tomcat", o);
        }
        if let Some(e) = child.stderr.take() {
            pipe_reader(app.clone(), i, "tomcat", e);
        }
        r.inner.lock().unwrap().child = Some(child);
        emit_state(app);
        watch_exit(app, i, gate);
    });
    Ok(())
}

// 끝날 때까지 지켜본다. 차례로 띄우기의 문(gate)은 「켜는 중」을 벗어나는 순간 놓아 다음 서버가 뜨게 한다.
fn watch_exit(app: &AppHandle, i: usize, mut gate: Option<std::sync::MutexGuard<'_, ()>>) {
    let r = rt(app, i);
    loop {
        thread::sleep(Duration::from_millis(400));
        if gate.is_some() && r.inner.lock().unwrap().status != Status::Starting {
            gate = None;
        }
        let mut inner = r.inner.lock().unwrap();
        let Some(child) = inner.child.as_mut() else { return };
        if let Ok(Some(code)) = child.try_wait() {
            let was_starting = inner.status == Status::Starting;
            inner.child = None;
            inner.status = Status::Stopped;
            inner.started_at_ms = None;
            inner.mem = Mem::default();
            drop(inner);
            *r.console.lock().unwrap() = None;
            if was_starting {
                log(app, Some(i), "app", "err", format!("톰캣이 다 뜨기 전에 끝났습니다 ({code}). 톰캣 탭의 마지막 줄과 logs/console.log 를 보세요."));
            } else {
                log(app, Some(i), "app", "", format!("톰캣이 끝났습니다 ({code})."));
            }
            emit_state(app);
            return;
        }
    }
}

// 켜는 도중 오래 멈춰 있던 톰캣을 끌 때, 끄기 전에 스레드 상태를 남긴다. 어디서 멈췄는지 볼 단서가 이것뿐이다.
fn dump_threads(app: &AppHandle, i: usize, java: &str, pid: u32, base: &str) {
    let jstack = Path::new(java).with_file_name("jstack.exe");
    if !jstack.exists() {
        return;
    }
    let out_path = Path::new(base).join("logs").join(format!("threads-{}.txt", now_ms()));
    let Ok(file) = File::create(&out_path) else { return };
    let spawned = Command::new(&jstack)
        .args(["-l", &pid.to_string()])
        .stdout(Stdio::from(file))
        .stderr(Stdio::null())
        .creation_flags(CREATE_NO_WINDOW)
        .spawn();
    let Ok(mut child) = spawned else { return };
    // 멈춘 JVM 에는 jstack 도 붙지 못할 수 있다. 10초만 기다린다.
    let until = Instant::now() + Duration::from_secs(10);
    while Instant::now() < until {
        if let Ok(Some(_)) = child.try_wait() {
            log(app, Some(i), "app", "warn", format!("켜는 도중 멈춰 있어서 끄기 전 스레드 상태를 남겼습니다: {}", p(&out_path)));
            return;
        }
        thread::sleep(Duration::from_millis(200));
    }
    let _ = child.kill();
}

// 종료 포트로 SHUTDOWN 을 보내 정상 종료를 기다리고, 20초 안에 안 끝나면 강제로 끈다.
// 기동 중에는 종료 포트가 아직 열리지 않으므로 바로 강제 종료로 간다.
fn stop_blocking(app: &AppHandle, i: usize) {
    let (cfg, sv) = cfgs(app, i);
    let r = rt(app, i);
    let (pid, java, starting_for) = {
        let mut inner = r.inner.lock().unwrap();
        if inner.child.is_none() {
            inner.status = Status::Stopped;
            drop(inner);
            emit_state(app);
            return;
        }
        let pid = inner.child.as_ref().map_or(0, |c| c.id());
        let starting_for = (inner.status == Status::Starting).then(|| now_ms().saturating_sub(inner.started_at_ms.unwrap_or(0)));
        inner.status = Status::Stopping;
        (pid, inner.java.clone(), starting_for)
    };
    emit_state(app);
    if starting_for.map_or(false, |ms| ms > 20_000) {
        dump_threads(app, i, &java, pid, &sv.catalina_base);
    }
    let addr = SocketAddr::from(([127, 0, 0, 1], sv.shutdown_port));
    let asked_ms = now_ms();
    let graceful = TcpStream::connect_timeout(&addr, Duration::from_millis(500))
        .and_then(|mut t| t.write_all(b"SHUTDOWN"))
        .is_ok();
    if graceful {
        log(app, Some(i), "app", "", "톰캣에 종료를 요청했습니다.");
        let until = Instant::now() + Duration::from_secs(20);
        while Instant::now() < until {
            if r.inner.lock().unwrap().child.is_none() {
                // 켜는 중에 끈 것이면 저장할 세션이 원래 없다
                if cfg.keep_session && starting_for.is_none() {
                    report_session_save(app, i, &sv.catalina_base, asked_ms);
                }
                return;
            }
            thread::sleep(Duration::from_millis(200));
        }
    }
    let mut inner = r.inner.lock().unwrap();
    if let Some(child) = inner.child.as_mut() {
        let _ = child.kill();
        let _ = child.wait();
        inner.child = None;
        inner.status = Status::Stopped;
        inner.started_at_ms = None;
        inner.mem = Mem::default();
        drop(inner);
        *r.console.lock().unwrap() = None;
        // 강제로 끄면 톰캣이 세션을 저장하지 못한다 — 켜져 있던 서버였으면 다음 기동에서 로그인이 풀린다고 알린다
        let lost = if cfg.keep_session && starting_for.is_none() { " 로그인 세션은 저장되지 않았습니다." } else { "" };
        log(app, Some(i), "app", "warn", format!("톰캣을 강제로 껐습니다.{lost}"));
        emit_state(app);
    }
}

/* ---------- 컴파일 ---------- */

fn collect_java(dir: &Path, since_ms: u64, out: &mut Vec<PathBuf>) {
    let Ok(rd) = fs::read_dir(dir) else { return };
    for e in rd.filter_map(Result::ok) {
        let path = e.path();
        let Ok(meta) = e.metadata() else { continue };
        if meta.is_dir() {
            collect_java(&path, since_ms, out);
        } else if path.extension().map_or(false, |x| x == "java") {
            let m = meta.modified().ok().and_then(|t| t.duration_since(UNIX_EPOCH).ok()).map(|d| d.as_millis() as u64).unwrap_or(u64::MAX);
            if m > since_ms {
                out.push(path);
            }
        }
    }
}

fn run_compile(app: &AppHandle, i: usize, full: bool) -> Option<CompileResult> {
    let (cfg, sv) = cfgs(app, i);
    let r = rt(app, i);
    let _guard = r.compile_lock.lock().unwrap();
    let classes = PathBuf::from(&sv.classes_dir);
    let stamp = read_stamp(&sv.classes_dir);
    let full = full || stamp == 0;
    let begin_ms = now_ms();
    let began = Instant::now();

    let src = Path::new(&sv.project).join("src/main/java");
    let mut files = Vec::new();
    collect_java(&src, if full { 0 } else { stamp }, &mut files);
    if files.is_empty() {
        log(app, Some(i), "compile", "", "바뀐 자바 파일이 없습니다.");
        return None;
    }
    r.inner.lock().unwrap().compiling = true;
    emit_state(app);

    // 톰캣이 꺼져 있을 때만 비우고 전체를 다시 만든다 (지운 소스의 클래스가 남지 않게).
    // 켜져 있을 때 비우면 실행 중인 앱이 클래스를 못 찾는다.
    let stopped = r.inner.lock().unwrap().status == Status::Stopped;
    if full && stopped {
        let _ = fs::remove_dir_all(&classes);
    }
    let _ = fs::create_dir_all(&classes);

    // javac 인자 파일 안에서 역슬래시는 이스케이프로 읽히므로 / 로 적는다
    let argfile = Path::new(&sv.catalina_base).join("temp/javac-files.txt");
    let _ = fs::create_dir_all(argfile.parent().unwrap());
    let list: String = files.iter().map(|f| format!("\"{}\"\n", p(f))).collect();
    let _ = fs::write(&argfile, list);

    let cp = build_cp(&cfg, &sv);
    log(
        app,
        Some(i),
        "compile",
        "",
        if full { format!("전체 컴파일 시작 · {}개 파일", files.len()) } else { format!("컴파일 시작 · {}", names(&files)) },
    );
    // 띄워 둔 상주 컴파일러가 먼저다. 없거나 응답이 없으면 javac 를 새로 띄워 같은 일을 한다.
    let request = format!("COMPILE\t{}\t{}\t{}", sv.classes_dir, cp, p(&argfile));
    let on_diag = |t: String| {
        let kind = if t.contains(": error:") || t.contains(": 오류:") { "err" } else { classify(&t) };
        log(app, Some(i), "compile", kind, t);
    };
    let (ok, errors, via) = match daemon_call(app, &request, on_diag) {
        Some((ok, errors)) => (ok, errors, format!("상주 {}", app.state::<Shared>().daemon_kind.lock().unwrap())),
        None => {
            let (ok, errors) = javac_process(app, i, &cfg, &sv, &cp, &argfile);
            (ok, errors, "javac 프로세스".to_string())
        }
    };
    if ok {
        // 시작 시각을 남긴다 — 컴파일하는 동안 고친 파일은 다음에 다시 잡힌다
        let _ = fs::write(classes.join(STAMP_FILE), begin_ms.to_string());
    }
    let result = CompileResult { ok, full, files: files.len(), errors, ms: began.elapsed().as_millis() as u64, at_ms: now_ms() };
    let secs = result.ms as f64 / 1000.0;
    if result.ok {
        let (running, hot) = {
            let inner = r.inner.lock().unwrap();
            (inner.status == Status::Running, inner.mode.starts_with("핫리로드"))
        };
        let tail = if running && hot {
            " · 실행 중인 앱에 넣는 중"
        } else if running {
            " · 일반 모드라 재시작해야 반영됩니다"
        } else {
            ""
        };
        if running && !hot {
            for f in &files {
                add_pending(app, i, rel_to(&sv.project, f), "일반 모드라 자바는 재시작해야 들어감");
            }
        }
        log(app, Some(i), "compile", "ok", format!("컴파일 완료 · {}개 · {secs:.1}초 ({via}){tail}", result.files));
    } else {
        log(app, Some(i), "compile", "err", format!("컴파일 실패 · 오류 {}개 · {secs:.1}초 · 오류 줄을 누르면 편집기로 엽니다", result.errors));
    }
    {
        let mut inner = r.inner.lock().unwrap();
        inner.compiling = false;
        inner.last_compile = Some(result.clone());
    }
    emit_state(app);
    Some(result)
}

fn build_cp(cfg: &Config, sv: &ServerCfg) -> String {
    format!("{};{}/*;{}/src/main/webapp/WEB-INF/lib/*;{}/lib/*", sv.classes_dir, sv.deps_dir, sv.project, cfg.tomcat_home)
}

fn spawn_daemon(app: &AppHandle) -> Option<Daemon> {
    let cfg = app.state::<Shared>().cfg.lock().unwrap().clone();
    let helper = helper_dir();
    if !helper.join("TomcatrunCompiler.class").exists() {
        return None;
    }
    // javac 쪽 컴파일러 API 는 JDK 의 tools.jar 에 있다. JRE 의 java 로는 뜨지 않는다.
    let bin = Path::new(&cfg.javac).parent()?;
    let tools = bin.parent()?.join("lib/tools.jar");
    // ECJ 는 톰캣 lib 의 jar 를 그대로 쓴다. 없으면 상주 컴파일러가 스스로 javac 로 간다.
    let ecj = if cfg.compiler == "javac" { None } else { ecj_jar(&cfg) };
    let mut cp = vec![p(&helper)];
    if let Some(j) = &ecj {
        cp.push(p(j));
    }
    cp.push(p(&tools));
    let mut child = Command::new(bin.join("java.exe"))
        .args(DAEMON_JVM_ARGS)
        .arg(format!("-Dtomcatrun.compiler={}", if ecj.is_some() { "ecj" } else { "javac" }))
        .arg("-cp")
        .arg(cp.join(";"))
        .arg("TomcatrunCompiler")
        .current_dir(home_dir())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .creation_flags(CREATE_NO_WINDOW)
        .spawn()
        .ok()?;
    assign_job(app.state::<Shared>().job, &child);
    let stdin = child.stdin.take()?;
    let stdout = BufReader::new(child.stdout.take()?);
    *app.state::<Shared>().daemon_kind.lock().unwrap() = if ecj.is_some() { "ECJ".into() } else { "javac".into() };
    Some(Daemon { child, stdin, stdout, uses: 0 })
}

fn stop_daemon(app: &AppHandle, why: &str) {
    let s = app.state::<Shared>();
    let had = match s.daemon.lock().unwrap().take() {
        Some(mut d) => {
            let _ = d.child.kill();
            true
        }
        None => false,
    };
    s.daemon_alive.store(false, Ordering::Relaxed);
    *s.daemon_mem.lock().unwrap() = Mem::default();
    if had && !why.is_empty() {
        log(app, None, "app", "", why.to_string());
    }
}

// 오래 안 쓰면 상주 컴파일러를 내린다. 컴파일 중이면(잠겨 있으면) 건너뛴다.
fn reap_idle_daemon(app: &AppHandle) {
    let s = app.state::<Shared>();
    let idle_ms = now_ms().saturating_sub(s.daemon_last_use.load(Ordering::Relaxed));
    if !s.daemon_alive.load(Ordering::Relaxed) || idle_ms < DAEMON_IDLE_MINUTES * 60_000 {
        return;
    }
    let Ok(mut guard) = s.daemon.try_lock() else { return };
    if let Some(mut d) = guard.take() {
        let _ = d.child.kill();
        drop(guard);
        s.daemon_alive.store(false, Ordering::Relaxed);
        *s.daemon_mem.lock().unwrap() = Mem::default();
        log(app, None, "app", "", format!("상주 컴파일러를 쉬게 했습니다 ({DAEMON_IDLE_MINUTES}분 동안 저장이 없었음). 다음 저장 때 다시 띄웁니다."));
        emit_state(app);
    }
}

// 상주 컴파일러에 한 줄을 보내고 END 까지 읽는다. 어떤 이유로든 실패하면 None — 부르는 쪽이 javac 로 대신한다.
fn daemon_call(app: &AppHandle, request: &str, mut on_diag: impl FnMut(String)) -> Option<(bool, usize)> {
    let s = app.state::<Shared>();
    let mut guard = s.daemon.lock().unwrap();
    if guard.as_ref().map_or(false, |d| d.uses >= DAEMON_MAX_USES) {
        if let Some(mut d) = guard.take() {
            let _ = d.child.kill();
        }
    }
    if guard.is_none() {
        *guard = spawn_daemon(app);
        s.daemon_alive.store(guard.is_some(), Ordering::Relaxed);
    }
    let d = guard.as_mut()?;
    d.uses += 1;
    s.daemon_last_use.store(now_ms(), Ordering::Relaxed);
    let res = (|| -> std::io::Result<(bool, usize)> {
        writeln!(d.stdin, "{request}")?;
        d.stdin.flush()?;
        loop {
            let mut buf = Vec::new();
            if d.stdout.read_until(b'\n', &mut buf)? == 0 {
                return Err(std::io::Error::new(std::io::ErrorKind::UnexpectedEof, "상주 컴파일러가 끝났습니다"));
            }
            let line = String::from_utf8_lossy(&buf).trim_end_matches(['\r', '\n']).to_string();
            if let Some(rest) = line.strip_prefix("D\t") {
                on_diag(rest.to_string());
            } else if let Some(rest) = line.strip_prefix("END\t") {
                let c: Vec<&str> = rest.split('\t').collect();
                return Ok((c.first() == Some(&"true"), c.get(1).and_then(|x| x.parse().ok()).unwrap_or(0)));
            }
        }
    })();
    match res {
        Ok(r) => Some(r),
        Err(e) => {
            if let Some(mut d) = guard.take() {
                let _ = d.child.kill();
            }
            s.daemon_alive.store(false, Ordering::Relaxed);
            drop(guard);
            log(app, None, "app", "warn", format!("상주 컴파일러가 응답하지 않아 javac 프로세스로 대신합니다 ({e}). 다음 저장 때 다시 띄웁니다."));
            None
        }
    }
}

fn javac_process(app: &AppHandle, i: usize, cfg: &Config, sv: &ServerCfg, cp: &str, argfile: &Path) -> (bool, usize) {
    // -g : Spring 이 @RequestParam 이름을 지역 변수 정보에서 읽는다. 빼면 이름 없는 파라미터가 깨진다.
    let out = Command::new(&cfg.javac)
        .args(["-nowarn", "-g", "-encoding", "UTF-8", "-source", "1.8", "-target", "1.8", "-implicit:none", "-sourcepath", ""])
        .arg("-d")
        .arg(&sv.classes_dir)
        .arg("-cp")
        .arg(cp)
        .arg(format!("@{}", p(argfile)))
        .current_dir(&sv.project)
        .creation_flags(CREATE_NO_WINDOW)
        .output();
    match out {
        Ok(o) => {
            let mut errors = 0;
            for chunk in [&o.stdout, &o.stderr] {
                for line in chunk.split(|b| *b == b'\n') {
                    let t = decode(line);
                    if t.is_empty() || t.starts_with("Note:") || t.starts_with("참고:") {
                        continue;
                    }
                    let kind = if t.contains(": error:") || t.contains(": 오류:") {
                        errors += 1;
                        "err"
                    } else {
                        classify(&t)
                    };
                    log(app, Some(i), "compile", kind, t);
                }
            }
            (o.status.success(), errors)
        }
        Err(e) => {
            log(app, Some(i), "compile", "err", format!("javac 를 실행하지 못했습니다: {e}"));
            (false, 1)
        }
    }
}

fn newest_java(dir: &Path, best: &mut Option<(u64, PathBuf)>) {
    let Ok(rd) = fs::read_dir(dir) else { return };
    for e in rd.filter_map(Result::ok) {
        let path = e.path();
        let Ok(meta) = e.metadata() else { continue };
        if meta.is_dir() {
            newest_java(&path, best);
        } else if path.extension().map_or(false, |x| x == "java") {
            let m = meta.modified().ok().and_then(|t| t.duration_since(UNIX_EPOCH).ok()).map(|d| d.as_millis() as u64).unwrap_or(0);
            if best.as_ref().map_or(true, |(b, _)| m > *b) {
                *best = Some((m, path));
            }
        }
    }
}

// 실행기를 켤 때 상주 컴파일러를 데운다. 작은 시험 파일로는 덜 데워져서(큰 파일 첫 컴파일 2.2초),
// 서버마다 가장 최근에 고친 실제 소스를 임시 폴더로 한 번 컴파일한다. 결과는 버리고 classes/ 는 건드리지 않는다.
// 코어가 4개뿐이라 톰캣 기동과 겹치면 둘 다 느려진다.
// 서버 하나가 다 뜬 뒤(켜는 중인 서버가 없을 때), 또는 60초 동안 아무 서버도 켜지 않았을 때 데운다.
// 2026-10-27 : 「켜고 5초 뒤」였는데, 실행기를 켜자마자 서버를 누르면 데우기가 서버 기동과 겹쳤다 (JVM · 에이전트 9.1초 · 톰캣 초기화 10.8초).
fn warm_daemon(app: &AppHandle) {
    let n = shared(app).rts.len();
    let began = Instant::now();
    loop {
        thread::sleep(Duration::from_secs(2));
        let statuses: Vec<Status> = (0..n).map(|i| rt(app, i).inner.lock().unwrap().status).collect();
        let starting = statuses.iter().any(|s| *s == Status::Starting);
        let running = statuses.iter().any(|s| *s == Status::Running);
        if !starting && (running || began.elapsed() >= Duration::from_secs(60)) {
            break;
        }
        if began.elapsed() >= Duration::from_secs(600) {
            return; // 10분 넘게 누가 켜는 중이면 데우지 않는다 (첫 저장 때 뜬다)
        }
    }
    // 그새 저장이 있어 이미 떴으면 데울 필요가 없다
    if app.state::<Shared>().daemon_alive.load(Ordering::Relaxed) {
        return;
    }
    let cfg = app.state::<Shared>().cfg.lock().unwrap().clone();
    let began = Instant::now();
    let tmp = helper_dir().join("warm");
    let _ = fs::create_dir_all(&tmp);
    for (i, sv) in cfg.servers.iter().enumerate() {
        let mut best = None;
        newest_java(&Path::new(&sv.project).join("src/main/java"), &mut best);
        let Some((_, file)) = best else { continue };
        let argfile = tmp.join(format!("args-{i}.txt"));
        if fs::write(&argfile, format!("\"{}\"\n", p(&file))).is_err() {
            continue;
        }
        let _guard = rt(app, i).compile_lock.lock().unwrap();
        if daemon_call(app, &format!("COMPILE\t{}\t{}\t{}", p(&tmp), build_cp(&cfg, sv), p(&argfile)), |_| {}).is_none() {
            return;
        }
    }
    emit_state(app);
    let kind = app.state::<Shared>().daemon_kind.lock().unwrap().clone();
    log(app, None, "app", "", format!("상주 컴파일러({kind})를 데웠습니다 ({:.1}초). 저장하면 이 컴파일러가 바로 받습니다.", began.elapsed().as_secs_f64()));
}

fn names(files: &[PathBuf]) -> String {
    let mut v: Vec<String> = files.iter().take(5).filter_map(|f| f.file_name().map(|n| n.to_string_lossy().into_owned())).collect();
    if files.len() > 5 {
        v.push(format!("외 {}개", files.len() - 5));
    }
    v.join(", ")
}

fn rel_to(project: &str, path: &Path) -> String {
    let full = p(path);
    let proj = project.trim_end_matches('/');
    if full.len() > proj.len() && full[..proj.len()].eq_ignore_ascii_case(proj) && full.as_bytes()[proj.len()] == b'/' {
        full[proj.len() + 1..].to_string()
    } else {
        full
    }
}

// 저장이 몰려 들어오면(여러 파일 저장 · 브랜치 전환) 잠시 조용해질 때까지 모았다가 한 번에 컴파일한다
fn compile_worker(app: AppHandle, i: usize, rx: Receiver<bool>) {
    loop {
        let Ok(first) = rx.recv() else { return };
        let mut full = first;
        loop {
            // 앞의 저장 감시가 이미 0.15초를 모아서 보내므로 여기서는 짧게 기다린다 (2026-10-27 : 0.3 → 0.1초)
            match rx.recv_timeout(Duration::from_millis(100)) {
                Ok(f) => full |= f,
                Err(RecvTimeoutError::Timeout) => break,
                Err(RecvTimeoutError::Disconnected) => return,
            }
        }
        run_compile(&app, i, full);
    }
}

/* ---------- 저장 감시 · 반영 분류 ---------- */

enum Change {
    Java,
    Live(&'static str, String),     // 다음 요청에 바로 들어간다
    Restart(String, &'static str), // 기동 때만 읽는다
    Pom,
}

// 앱이 스스로 다시 읽는 매퍼 묶음. context-datasource.xml 에서 RefreshableSqlSessionFactoryBean 의 mapperLocations 를 읽는다.
// 프로젝트마다 다르고 앞으로도 바뀔 수 있어 코드에 적지 않는다.
fn parse_live_mappers(xml: &str) -> Vec<String> {
    // 주석으로 막아 둔 빈은 없는 것이다
    let mut text = String::new();
    let mut rest = xml;
    while let Some(a) = rest.find("<!--") {
        text.push_str(&rest[..a]);
        rest = match rest[a..].find("-->") {
            Some(b) => &rest[a + b + 3..],
            None => "",
        };
    }
    text.push_str(rest);
    let mut out = Vec::new();
    for bean in text.split("<bean").skip(1) {
        let head = bean.split('>').next().unwrap_or("");
        if !head.contains("RefreshableSqlSessionFactoryBean") {
            continue;
        }
        // mapperLocations 값만 본다. 같은 빈의 configLocation(sql-mapper-config.xml)은 매퍼가 아니다.
        //   p:mapperLocations="…"  또는  <property name="mapperLocations"> <list><value>…</value></list> </property>
        let Some(at) = bean.find("mapperLocations") else { continue };
        let after = &bean[at + "mapperLocations".len()..];
        let mut s = match after.strip_prefix("=\"") {
            Some(v) => &v[..v.find('"').unwrap_or(v.len())],
            None => &after[..after.find("</property>").unwrap_or(after.len())],
        };
        while let Some(a) = s.find("classpath") {
            s = &s[a..];
            let Some(colon) = s.find(":/") else { break };
            let body = &s[colon + 2..];
            let end = body.find(|c: char| c == '"' || c == '<' || c == ',' || c.is_whitespace()).unwrap_or(body.len());
            let pat = &body[..end];
            // 「sqlmap/app/**/**/*.xml」 → 「sqlmap/app/」, 별표가 없으면 파일 하나
            let prefix = pat.split('*').next().unwrap_or("").to_ascii_lowercase();
            if !prefix.is_empty() {
                out.push(prefix);
            }
            s = &body[end..];
        }
    }
    out
}

fn live_mappers(project: &str) -> Vec<String> {
    fs::read_to_string(Path::new(project).join("src/main/resources/spring/context-datasource.xml"))
        .map(|x| parse_live_mappers(&x))
        .unwrap_or_default()
}

fn classify_path(project: &str, live: &[String], path: &Path) -> Option<Change> {
    let rel = rel_to(project, path);
    let name = rel.rsplit('/').next().unwrap_or("").to_string();
    // 편집기 임시 파일 · 폴더 이벤트는 거른다
    // 2026-10-28 : 원자적 저장(「a.jsp.tmp.<pid>.<해시>」에 쓰고 이름을 바꾼다)의 중간 파일은 .tmp 로 끝나지 않아
    //              WEB-INF 아래에서 「WEB-INF 설정 · 재시작 필요」로 잡혔다
    if name.starts_with('.') || name.ends_with('~') || name.ends_with(".tmp") || name.contains(".tmp.") || name.ends_with(".swp") || !name.contains('.') {
        return None;
    }
    let lower = rel.to_ascii_lowercase();
    if lower == "pom.xml" {
        return Some(Change::Pom);
    }
    if let Some(r) = lower.strip_prefix("src/main/java/") {
        return r.ends_with(".java").then_some(Change::Java);
    }
    if let Some(r) = lower.strip_prefix("src/main/webapp/") {
        if let Some(w) = r.strip_prefix("web-inf/") {
            if [".jsp", ".jspf", ".tag", ".tagx"].iter().any(|e| w.ends_with(e)) {
                return Some(Change::Live("화면", name));
            }
            let why = if w.starts_with("lib/") { "jar" } else { "WEB-INF 설정" };
            return Some(Change::Restart(rel, why));
        }
        return Some(Change::Live("화면", name));
    }
    if let Some(r) = lower.strip_prefix("src/main/resources/") {
        // 다른 프로필용 폴더와 옛 사본은 로컬 실행에 쓰이지 않는다
        // (위 임시 파일 규칙과 함께 설정 화면 「저장 감시 제외」의 「기본」 줄에 적혀 있다 · ui/index.html 의 IGNORE_BUILTIN)
        if r.starts_with("resources-") || r.starts_with("java/") || r.starts_with("webapp/") {
            return None;
        }
        if r.ends_with(".xml") && (r.starts_with("sqlmap/") || r.starts_with("sqlmap-")) {
            if live.iter().any(|m| r.starts_with(m.as_str())) {
                return Some(Change::Live("매퍼", name));
            }
            return Some(Change::Restart(rel, "자동으로 다시 읽지 않는 매퍼"));
        }
        let why = if r.starts_with("message/") {
            "메시지 (캐시를 비우지 않음)"
        } else if r.starts_with("spring/") || r.starts_with("egovframework/") {
            "Spring 설정"
        } else {
            "설정"
        };
        return Some(Change::Restart(rel, why));
    }
    None
}

/// 무시 목록에 걸리는지 (rel : 프로젝트 기준 경로). 대소문자 · 역슬래시는 가리지 않는다.
///   「/」 없는 패턴(*.sql · deploy)   → 경로 어느 단계의 이름이든 맞으면 (그 폴더 아래 전부 포함)
///   「/」 있는 패턴(src/main/x · src/**/tmp) → 프로젝트 맨 위부터. 폴더가 맞으면 그 아래 전부
/// 「*」 는 「/」 를 넘지 않고, 「**」 는 넘는다 (「**/」 는 폴더 0개도 받는다).
fn ignored(patterns: &[String], rel: &str) -> bool {
    let rel = rel.replace('\\', "/").to_ascii_lowercase();
    let segs: Vec<&str> = rel.split('/').filter(|s| !s.is_empty()).collect();
    patterns.iter().any(|raw| {
        let pat = norm_pattern(raw);
        if pat.is_empty() {
            return false;
        }
        if !pat.contains('/') {
            return segs.iter().any(|s| glob(&pat, s));
        }
        (1..=segs.len()).any(|n| glob(&pat, &segs[..n].join("/")))
    })
}

/// 화면에서 받은 무시 목록을 저장할 꼴로 다듬는다 — 빈 줄 · 같은 패턴(대소문자 · 역슬래시만 다른 것)은 하나로.
/// 탐색기 「경로로 복사」 는 따옴표를 붙이고 절대 경로를 주므로, 따옴표를 떼고 프로젝트 기준으로 바꾼다.
fn clean_ignore(projects: &[String], list: Vec<String>) -> Vec<String> {
    let projects: Vec<String> = projects.iter().map(|p| p.replace('\\', "/").trim_end_matches('/').to_ascii_lowercase()).collect();
    let mut out: Vec<String> = Vec::new();
    for raw in list {
        let mut x = raw.trim().trim_matches('"').trim().replace('\\', "/");
        let low = x.to_ascii_lowercase();
        if let Some(pj) = projects.iter().find(|pj| low.starts_with(&format!("{pj}/"))) {
            x = x[pj.len() + 1..].to_string();
        }
        if norm_pattern(&x).is_empty() || out.iter().any(|o| norm_pattern(o) == norm_pattern(&x)) {
            continue;
        }
        out.push(x);
    }
    out
}

fn norm_pattern(raw: &str) -> String {
    let mut s = raw.trim().replace('\\', "/").to_ascii_lowercase();
    while let Some(rest) = s.strip_prefix("./") {
        s = rest.to_string();
    }
    s.trim_matches('/').to_string()
}

fn glob(pat: &str, text: &str) -> bool {
    fn m(p: &[char], t: &[char]) -> bool {
        match p.first() {
            None => t.is_empty(),
            Some('*') if p.get(1) == Some(&'*') => {
                if p.get(2) == Some(&'/') {
                    (0..=t.len()).any(|k| (k == 0 || t[k - 1] == '/') && m(&p[3..], &t[k..]))
                } else {
                    (0..=t.len()).any(|k| m(&p[2..], &t[k..]))
                }
            }
            Some('*') => {
                let mut k = 0;
                loop {
                    if m(&p[1..], &t[k..]) {
                        return true;
                    }
                    if k == t.len() || t[k] == '/' {
                        return false;
                    }
                    k += 1;
                }
            }
            Some('?') => !t.is_empty() && t[0] != '/' && m(&p[1..], &t[1..]),
            Some(c) => t.first() == Some(c) && m(&p[1..], &t[1..]),
        }
    }
    let p: Vec<char> = pat.chars().collect();
    let t: Vec<char> = text.chars().collect();
    m(&p, &t)
}

fn change_worker(app: AppHandle, rx: Receiver<(usize, PathBuf)>) {
    loop {
        let Ok(first) = rx.recv() else { return };
        let mut paths: HashSet<(usize, PathBuf)> = HashSet::from([first]);
        loop {
            // 편집기 한 번 저장에 이벤트가 2~3개(임시 파일 · 이름 바꾸기 · 수정) 몰려 온다. 그 사이만 모은다 (2026-10-27 : 0.4 → 0.15초)
            match rx.recv_timeout(Duration::from_millis(150)) {
                Ok(x) => {
                    paths.insert(x);
                }
                Err(RecvTimeoutError::Timeout) => break,
                Err(RecvTimeoutError::Disconnected) => return,
            }
        }
        let count = app.state::<Shared>().rts.len();
        for i in 0..count {
            let mine: Vec<&PathBuf> = paths.iter().filter(|(n, _)| *n == i).map(|(_, p)| p).collect();
            if !mine.is_empty() {
                apply_changes(&app, i, &mine);
                // 저장이 있었으니 버전 관리 화면의 바뀐 파일 목록도 달라졌을 수 있다
                let _ = app.emit("vcs-dirty", i);
            }
        }
        emit_state(&app);
    }
}

fn apply_changes(app: &AppHandle, i: usize, paths: &[&PathBuf]) {
    let (c, sv) = cfgs(app, i);
    // 무시 목록에 걸린 파일은 반영 기록 · 재시작 알림 · 자동 컴파일 어디에도 넣지 않는다 (SVN 목록에는 그대로 나온다)
    let paths: Vec<&PathBuf> = paths.iter().copied().filter(|p| !ignored(&c.ignore, &rel_to(&sv.project, p))).collect();
    if paths.is_empty() {
        return;
    }
    let r = rt(app, i);
    let live_list = r.live_mappers.lock().unwrap().clone();
    let running = r.inner.lock().unwrap().status != Status::Stopped;
    let (mut java, mut pom) = (false, false);
    let mut live: Vec<String> = Vec::new();
    let mut mappers: Vec<String> = Vec::new();
    let mut restart: Vec<String> = Vec::new();
    for path in paths {
        match classify_path(&sv.project, &live_list, path) {
            Some(Change::Java) => java = true,
            Some(Change::Pom) => pom = true,
            Some(Change::Live("매퍼", name)) => mappers.push(name),
            Some(Change::Live(what, name)) => live.push(format!("{what} {name}")),
            Some(Change::Restart(rel, why)) => {
                if running {
                    add_pending(app, i, rel.clone(), why);
                }
                restart.push(rel);
            }
            None => {}
        }
    }
    if java && app.state::<Shared>().auto_compile.load(Ordering::Relaxed) {
        let _ = r.compile_tx.send(false);
    }
    if running && !live.is_empty() {
        live.sort();
        log(app, Some(i), "app", "live", format!("바로 반영 · {}", live.join(", ")));
    }
    if running && !mappers.is_empty() {
        mappers.sort();
        let names = mappers.join(", ");
        let hotswap = {
            let mut inner = r.inner.lock().unwrap();
            if inner.mapper_hotswap {
                inner.mapper_wait = Some((now_ms(), names.clone()));
            }
            inner.mapper_hotswap
        };
        // 앱의 RefreshableSqlSessionFactoryBean 은 바뀐 파일 하나가 아니라 묶음 전체(매퍼 112개 묶음 기준)를 다시 해석한다 (2026-10-27 실측 2~2.5초)
        let how = if hotswap { "바뀐 파일만 다시 읽는 중" } else { "묶음 전체를 다시 해석하는 중 · 2~3초" };
        log(app, Some(i), "app", "live", format!("매퍼 {how} · {names}"));
    }
    if running && !restart.is_empty() {
        restart.sort();
        log(app, Some(i), "app", "warn", format!("재시작해야 반영 · {}", restart.join(", ")));
    }
    if pom {
        log(app, Some(i), "app", "warn", "pom.xml 이 바뀌었습니다. 의존성이 달라졌으면 「의존 jar 다시 받기」를 누르세요.");
    }
}

fn start_watchers(app: &AppHandle) {
    let s = app.state::<Shared>();
    let servers = s.cfg.lock().unwrap().servers.clone();
    for (i, sv) in servers.iter().enumerate() {
        *s.rts[i].live_mappers.lock().unwrap() = live_mappers(&sv.project);
        let project = PathBuf::from(&sv.project);
        let tx = s.change_tx.clone();
        let watcher = notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
            let Ok(ev) = res else { return };
            if !matches!(ev.kind, EventKind::Create(_) | EventKind::Modify(_) | EventKind::Remove(_)) {
                return;
            }
            for path in ev.paths {
                let _ = tx.send((i, path));
            }
        });
        let mut w = match watcher {
            Ok(w) => w,
            Err(e) => {
                log(app, Some(i), "app", "err", format!("파일 감시를 시작하지 못했습니다: {e}"));
                continue;
            }
        };
        // src/main 전체와 pom.xml (프로젝트 루트는 한 단계만)
        let main = project.join("src/main");
        match w.watch(&main, RecursiveMode::Recursive).and_then(|_| w.watch(&project, RecursiveMode::NonRecursive)) {
            Ok(()) => {
                log(app, Some(i), "app", "", format!("저장 감시 중: {}", p(&main)));
                s.watchers.lock().unwrap().push(w);
            }
            Err(e) => log(app, Some(i), "app", "err", format!("소스 폴더를 감시하지 못했습니다: {e}")),
        }
    }
}

/* ---------- 의존 jar ---------- */

// pom 기준으로 deps.new 에 받아 두고, 다 받은 뒤에만 deps 와 바꾼다. 중간에 실패해도 쓰던 jar 는 그대로다.
fn refresh_deps_blocking(app: &AppHandle, i: usize) -> bool {
    let (cfg, sv) = cfgs(app, i);
    let deps = PathBuf::from(&sv.deps_dir);
    let fresh = deps.with_file_name("deps.new");
    let old = deps.with_file_name("deps.old");
    let _ = fs::remove_dir_all(&fresh);
    let java_home = Path::new(&cfg.javac).parent().and_then(Path::parent).map(Path::to_path_buf).unwrap_or_default();
    // 원본 프로젝트 폴더에는 아무것도 쓰지 않는다 — pom 을 실행기 폴더로 복사해 거기서 돌린다 (Maven 이 target/ 등을 만들 수 있다).
    // 사본에서도 소스를 가리키는 경로(systemPath 의 hwplib 등)는 원본을 보게 바꾸고, 빌드 폴더(target)는 사본 쪽에 둔다.
    let mvn_dir = Path::new(&sv.catalina_base).join("maven");
    let _ = fs::create_dir_all(&mvn_dir);
    let pom = match fs::read_to_string(Path::new(&sv.project).join("pom.xml")) {
        Ok(t) => t,
        Err(e) => {
            log(app, Some(i), "deps", "err", format!("pom.xml 을 읽지 못했습니다: {e}"));
            return false;
        }
    };
    let proj = sv.project.trim_end_matches('/');
    let pom = pom.replace("${project.basedir}/src", &format!("{proj}/src")).replace("${basedir}/src", &format!("{proj}/src"));
    if let Err(e) = fs::write(mvn_dir.join("pom.xml"), pom) {
        log(app, Some(i), "deps", "err", format!("pom.xml 사본을 쓰지 못했습니다: {e}"));
        return false;
    }
    log(app, Some(i), "deps", "", "의존 jar 를 pom.xml 기준으로 다시 받습니다 (Maven · 사본에서 실행).");
    let spawned = Command::new(&cfg.maven)
        .args(["-B", "-f"])
        .arg(p(&mvn_dir.join("pom.xml")))
        .args(["dependency:copy-dependencies", "-DincludeScope=runtime"])
        .arg(format!("-DoutputDirectory={}", p(&fresh)))
        .current_dir(&mvn_dir)
        .env("JAVA_HOME", &java_home)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .creation_flags(CREATE_NO_WINDOW)
        .spawn();
    let mut child = match spawned {
        Ok(c) => c,
        Err(e) => {
            log(app, Some(i), "deps", "err", format!("Maven 을 실행하지 못했습니다: {e}"));
            return false;
        }
    };
    let readers: Vec<_> = [
        child.stdout.take().map(|o| pipe_reader(app.clone(), i, "deps", o)),
        child.stderr.take().map(|e| pipe_reader(app.clone(), i, "deps", e)),
    ]
    .into_iter()
    .flatten()
    .collect();
    let ok = child.wait().map(|s| s.success()).unwrap_or(false);
    for reader in readers {
        let _ = reader.join();
    }
    let jars = fs::read_dir(&fresh).map(|r| r.filter_map(Result::ok).count()).unwrap_or(0);
    if !ok || jars == 0 {
        log(app, Some(i), "deps", "err", "의존 jar 를 받지 못했습니다. 쓰던 jar 는 그대로 둡니다.");
        let _ = fs::remove_dir_all(&fresh);
        return false;
    }
    let _ = fs::remove_dir_all(&old);
    if let Err(e) = fs::rename(&deps, &old).and_then(|_| fs::rename(&fresh, &deps)) {
        log(app, Some(i), "deps", "err", format!("jar 폴더를 바꾸지 못했습니다: {e}"));
        if !deps.exists() {
            let _ = fs::rename(&old, &deps);
        }
        return false;
    }
    let _ = fs::remove_dir_all(&old);
    let _ = fs::write(deps.join(STAMP_FILE), now_ms().to_string());
    // 상주 컴파일러는 jar 목록을 들고 있다. 바뀐 jar 를 보도록 내려 두고 다음 저장 때 새로 띄운다.
    stop_daemon(app, "");
    log(app, Some(i), "deps", "ok", format!("의존 jar {jars}개로 바꿨습니다. 다음 시작부터 씁니다."));
    true
}

/* ---------- 화면 명령 ---------- */

fn check_srv(app: &AppHandle, srv: usize) -> Result<(), String> {
    if srv < app.state::<Shared>().rts.len() {
        Ok(())
    } else {
        Err(format!("없는 서버: {srv}"))
    }
}

#[tauri::command]
fn get_state(app: AppHandle) -> Snapshot {
    snapshot(&app)
}

#[tauri::command]
fn start(app: AppHandle, srv: usize) -> Result<(), String> {
    check_srv(&app, srv)?;
    start_server(&app, srv)
}

#[tauri::command]
fn stop(app: AppHandle, srv: usize) -> Result<(), String> {
    check_srv(&app, srv)?;
    thread::spawn(move || stop_blocking(&app, srv));
    Ok(())
}

fn running_names(app: &AppHandle) -> Vec<String> {
    let c = shared(app).cfg.lock().unwrap().clone();
    (0..shared(app).rts.len())
        .filter(|&i| rt(app, i).inner.lock().unwrap().child.is_some())
        .map(|i| c.servers[i].name.clone())
        .collect()
}

// 닫기 확인 뒤 — 켜진 톰캣을 함께 정상 종료하고 실행기를 끝낸다.
// 실행기가 갑자기 죽는 경우는 작업 개체가 톰캣을 함께 끈다.
#[tauri::command]
fn quit(app: AppHandle) {
    quit_then(app, |_| {});
}

// 켜진 톰캣을 모두 정상 종료한 뒤 after 를 부르고 실행기를 끝낸다 (닫기 · 업데이트 설치)
fn quit_then(app: AppHandle, after: impl FnOnce(&AppHandle) + Send + 'static) {
    if shared(&app).quitting.swap(true, Ordering::Relaxed) {
        return;
    }
    let n = shared(&app).rts.len();
    let running: Vec<usize> = (0..n).filter(|&i| rt(&app, i).inner.lock().unwrap().child.is_some()).collect();
    if !running.is_empty() {
        log(&app, None, "app", "", "창을 닫기 전에 톰캣을 끕니다.");
    }
    thread::spawn(move || {
        let hs: Vec<_> = running
            .into_iter()
            .map(|i| {
                let a = app.clone();
                thread::spawn(move || stop_blocking(&a, i))
            })
            .collect();
        for h in hs {
            let _ = h.join();
        }
        after(&app);
        app.exit(0);
    });
}

#[tauri::command]
fn restart(app: AppHandle, srv: usize) -> Result<(), String> {
    check_srv(&app, srv)?;
    thread::spawn(move || {
        stop_blocking(&app, srv);
        if let Err(e) = start_server(&app, srv) {
            log(&app, Some(srv), "app", "err", e.clone());
            let _ = app.emit("error", e);
        }
    });
    Ok(())
}

#[tauri::command]
fn compile(app: AppHandle, srv: usize, full: bool) -> Result<(), String> {
    check_srv(&app, srv)?;
    let _ = rt(&app, srv).compile_tx.send(full);
    Ok(())
}

#[tauri::command]
fn refresh_deps(app: AppHandle, srv: usize) -> Result<(), String> {
    check_srv(&app, srv)?;
    let r = rt(&app, srv);
    let name = cfgs(&app, srv).1.name;
    if r.inner.lock().unwrap().status != Status::Stopped {
        return Err(format!("{name} 톰캣을 끈 뒤에 받으세요. 실행 중에는 jar 가 잠겨 바꿀 수 없습니다."));
    }
    let sv = cfgs(&app, srv).1;
    if sv.project.trim().is_empty() {
        return Err(format!("{name} 프로젝트 폴더를 먼저 정하세요 (「시작하기」 또는 설정 › 서버)."));
    }
    if !Path::new(&sv.project).join("pom.xml").is_file() {
        return Err(format!("{name} 프로젝트에 pom.xml 이 없어 Maven 으로 받을 수 없습니다: {}", sv.project));
    }
    if r.deps_busy.swap(true, Ordering::SeqCst) {
        return Err("이미 받는 중입니다.".into());
    }
    emit_state(&app);
    thread::spawn(move || {
        if !refresh_deps_blocking(&app, srv) {
            let _ = app.emit("error", format!(
                "{name} 의존 jar 를 받지 못했습니다. Maven 로그는 빌드 탭에 있습니다. 이클립스로 배포해 둔 적이 있으면 「시작하기 › 이클립스에서 가져오기」로 채울 수 있습니다."
            ));
        }
        rt(&app, srv).deps_busy.store(false, Ordering::SeqCst);
        emit_state(&app);
    });
    Ok(())
}

#[tauri::command]
fn set_option(app: AppHandle, key: String, value: bool) -> Result<(), String> {
    let s = app.state::<Shared>();
    {
        let mut c = s.cfg.lock().unwrap();
        match key.as_str() {
            "hotReload" => c.hot_reload = value,
            "autoCompile" => {
                c.auto_compile = value;
                s.auto_compile.store(value, Ordering::Relaxed);
            }
            "debug" => c.debug = value,
            "fastJvm" => c.fast_jvm = value,
            "staggerStart" => c.stagger_start = value,
            "skipScan" => c.skip_scan = value,
            "smallPool" => c.small_pool = value,
            "noSchedule" => c.no_schedule = value,
            "lazyInit" => c.lazy_init = value,
            "mapperHotswap" => c.mapper_hotswap = value,
            "keepSession" => c.keep_session = value,
            "updateCheck" => c.update_check = value,
            _ => return Err(format!("모르는 옵션: {key}")),
        }
        save_config(&c).map_err(|e| format!("설정을 저장하지 못했습니다: {e}"))?;
    }
    emit_state(&app);
    Ok(())
}

#[tauri::command]
fn set_heap(app: AppHandle, mb: u32) -> Result<(), String> {
    if mb != 0 && !(512..=8192).contains(&mb) {
        return Err(format!("힙 상한은 512~8192MB 또는 0(상한 없음)이어야 합니다: {mb}"));
    }
    {
        let mut c = shared(&app).cfg.lock().unwrap();
        c.heap_mb = mb;
        save_config(&c).map_err(|e| format!("설정을 저장하지 못했습니다: {e}"))?;
    }
    emit_state(&app);
    Ok(())
}

#[tauri::command]
fn set_compiler(app: AppHandle, name: String) -> Result<(), String> {
    if name != "ecj" && name != "javac" {
        return Err(format!("모르는 컴파일러: {name}"));
    }
    {
        let mut c = shared(&app).cfg.lock().unwrap();
        c.compiler = name.clone();
        save_config(&c).map_err(|e| format!("설정을 저장하지 못했습니다: {e}"))?;
    }
    // 떠 있는 상주 컴파일러를 내려 다음 저장 때 새 컴파일러로 띄운다
    stop_daemon(&app, &format!("컴파일러를 {name} 로 바꿨습니다. 다음 저장 때 새로 띄웁니다."));
    emit_state(&app);
    Ok(())
}

#[tauri::command]
fn set_ignore(app: AppHandle, list: Vec<String>) -> Result<(), String> {
    let s = shared(&app);
    let clean = {
        let mut c = s.cfg.lock().unwrap();
        let projects: Vec<String> = c.servers.iter().map(|sv| sv.project.clone()).collect();
        let out = clean_ignore(&projects, list);
        c.ignore = out.clone();
        save_config(&c).map_err(|e| format!("설정을 저장하지 못했습니다: {e}"))?;
        out
    };
    // 이미 쌓인 「재시작 필요」 가운데 이제 무시하는 것은 걷어낸다
    for i in 0..s.rts.len() {
        rt(&app, i).inner.lock().unwrap().restart_needed.retain(|x| !ignored(&clean, &x.path));
    }
    emit_state(&app);
    Ok(())
}

// 「설정 › 경로」 — 다음 기동 · 다음 컴파일부터 쓴다
#[tauri::command]
fn set_paths(app: AppHandle, paths: Paths) -> Result<(), String> {
    let tomcat = paths.tomcat_home.trim().replace('\\', "/").trim_end_matches('/').to_string();
    if !Path::new(&tomcat).join("bin/bootstrap.jar").exists() {
        return Err(format!("톰캣 폴더가 아닙니다 (bin/bootstrap.jar 가 없습니다): {tomcat}"));
    }
    let javac = paths.javac.trim().replace('\\', "/");
    let java = paths.java_plain.trim().replace('\\', "/");
    for (what, f) in [("javac", &javac), ("java", &java)] {
        if !Path::new(f).is_file() {
            return Err(format!("{what} 가 없습니다: {f}"));
        }
    }
    {
        let mut c = shared(&app).cfg.lock().unwrap();
        c.tomcat_home = tomcat;
        c.javac = javac;
        c.java_plain = java;
        c.editor = paths.editor.trim().to_string();
        c.tld_scan = paths.tld_scan.split([',', '\n']).map(str::trim).filter(|x| !x.is_empty()).collect::<Vec<_>>().join(",");
        save_config(&c).map_err(|e| format!("설정을 저장하지 못했습니다: {e}"))?;
    }
    emit_state(&app);
    Ok(())
}

// 「설정 › 서버」 — 포트 · 이름 · 열기 단추는 다음 기동부터, 프로젝트 폴더의 저장 감시는 실행기를 다시 열면 따라간다
#[tauri::command]
fn set_server(app: AppHandle, srv: usize, edit: ServerEdit) -> Result<bool, String> {
    check_srv(&app, srv)?;
    let project = edit.project.trim().replace('\\', "/").trim_end_matches('/').to_string();
    if !project.is_empty() && !Path::new(&project).join("src/main/webapp").is_dir() {
        return Err(format!("src/main/webapp 이 있는 프로젝트 폴더를 고르세요: {project}"));
    }
    let name = edit.name.trim();
    if name.is_empty() {
        return Err("서버 이름을 적으세요.".into());
    }
    let ports = [edit.http_port, edit.ajp_port, edit.shutdown_port, edit.debug_port];
    if ports.iter().any(|&x| x == 0) || ports.iter().collect::<HashSet<_>>().len() < ports.len() {
        return Err("포트 네 개는 모두 다른 0 아닌 수여야 합니다.".into());
    }
    let c = {
        let mut c = shared(&app).cfg.lock().unwrap();
        // 다른 서버와 포트가 겹치면 함께 띄울 수 없다
        for (i, o) in c.servers.iter().enumerate() {
            if i != srv && [o.http_port, o.ajp_port, o.shutdown_port, o.debug_port].iter().any(|x| ports.contains(x)) {
                return Err(format!("{} 와 포트가 겹칩니다.", o.name));
            }
        }
        let sv = &mut c.servers[srv];
        let moved = !sv.project.eq_ignore_ascii_case(&project);
        sv.name = name.to_string();
        sv.project = project;
        sv.http_port = edit.http_port;
        sv.ajp_port = edit.ajp_port;
        sv.shutdown_port = edit.shutdown_port;
        sv.debug_port = edit.debug_port;
        sv.links = edit.links.into_iter().filter(|l| !l.url.trim().is_empty()).map(|l| {
            let url = l.url.trim().to_string();
            Link { label: if l.label.trim().is_empty() { url.clone() } else { l.label.trim().to_string() }, url }
        }).collect();
        sv.jvm_args = edit.jvm_args.into_iter().map(|a| a.trim().to_string()).filter(|a| !a.is_empty()).collect();
        (c.clone(), moved)
    };
    save_config(&c.0).map_err(|e| format!("설정을 저장하지 못했습니다: {e}"))?;
    emit_state(&app);
    Ok(c.1)
}

// 서버를 더한다 — 서버마다 상태 · 감시 · 컴파일 줄이 실행기를 켤 때 만들어지므로, 실행기를 다시 열면 보인다
#[tauri::command]
fn add_server(app: AppHandle) -> Result<(), String> {
    let mut c = shared(&app).cfg.lock().unwrap().clone();
    let n = c.servers.len();
    let id = (n + 1..).map(|k| format!("app{k}")).find(|id| c.servers.iter().all(|s| &s.id != id)).unwrap_or_default();
    let mut sv = new_server(&home_dir(), &id, &format!("앱 {}", n + 1), n as u16);
    // 기본 포트가 이미 쓰이면 비어 있는 자리까지 올린다 — 다른 서버의 설정뿐 아니라 지금 이 PC 에서 듣고 있는 포트(IDE 톰캣 등)도 건너뛴다.
    // 열기 단추는 「/」라 포트를 따라가므로 다시 만들지 않는다.
    let used: HashSet<u16> = c.servers.iter().flat_map(|s| [s.http_port, s.ajp_port, s.shutdown_port, s.debug_port]).collect();
    let mut k = n as u16;
    while [sv.http_port, sv.ajp_port, sv.shutdown_port, sv.debug_port].iter().any(|&x| used.contains(&x) || port_busy(x)) {
        k += 1;
        let t = new_server(&home_dir(), &id, &sv.name, k);
        (sv.http_port, sv.ajp_port, sv.shutdown_port, sv.debug_port) = (t.http_port, t.ajp_port, t.shutdown_port, t.debug_port);
    }
    c.servers.push(sv);
    save_config(&c).map_err(|e| format!("설정을 저장하지 못했습니다: {e}"))
}

// 「설정 › 서버 › 프로젝트 폴더 › 고르기」 — Windows 폴더 고르기 창. 고른 폴더나 그 바로 아래에서 src/main/webapp 이 있는 폴더를 돌려준다.
// 빈 목록이면 취소, 둘 이상이면 화면이 하나를 고르게 한다. 저장은 하지 않는다 (칸에 넣고 「저장」은 사용자가 누른다).
// 창이 떠 있는 동안 막히므로 화면 스레드가 아닌 곳에서 띄운다.
#[tauri::command]
async fn pick_project(app: AppHandle, start: String) -> Result<Vec<String>, String> {
    use tauri_plugin_dialog::DialogExt;
    tauri::async_runtime::spawn_blocking(move || {
        let mut dlg = app.dialog().file().set_title("프로젝트 폴더 고르기 (src/main/webapp 이 있는 폴더)");
        let start = start.trim();
        if !start.is_empty() && Path::new(start).is_dir() {
            dlg = dlg.set_directory(start);
        }
        let Some(dir) = dlg.blocking_pick_folder().and_then(|f| f.into_path().ok()) else {
            return Ok(vec![]);
        };
        let found = webapp_roots(&dir);
        if found.is_empty() {
            return Err(format!("src/main/webapp 이 있는 프로젝트 폴더를 고르세요: {}", p(&dir)));
        }
        Ok(found)
    }).await.map_err(|e| e.to_string())?
}

/* ---------- 시작하기 (이클립스에서 가져오기 · 프로젝트 폴더 하나) ---------- */

// dir 이 있으면 그 작업 공간만, 없으면 흔한 자리와 지금 프로젝트들의 부모 폴더에서 찾는다
#[tauri::command]
async fn eclipse_scan(app: AppHandle, dir: Option<String>) -> Result<Vec<eclipse::Workspace>, String> {
    let projects: Vec<PathBuf> = shared(&app).cfg.lock().unwrap().servers.iter()
        .filter(|s| !s.project.is_empty())
        .filter_map(|s| Path::new(&s.project).parent().map(Path::to_path_buf))
        .collect();
    tauri::async_runtime::spawn_blocking(move || match dir {
        Some(d) => {
            let d = PathBuf::from(d.trim());
            if !eclipse::is_workspace(&d) {
                return Err(format!("이클립스 작업 공간이 아닙니다 (.metadata 의 서버 설정이 없습니다): {}", p(&d)));
            }
            Ok(vec![eclipse::read_workspace(&d)])
        }
        None => Ok(eclipse::find_workspaces(&projects).iter().map(|w| eclipse::read_workspace(w)).filter(|w| !w.servers.is_empty()).collect()),
    })
    .await
    .map_err(|e| e.to_string())?
}

// 고른 이클립스 서버를 실행기 서버로 — 같은 프로젝트를 쓰는 서버가 있으면 그것을, 프로젝트가 빈 서버가 있으면 그것을 채우고, 없으면 더한다.
// 의존 jar 는 이클립스 배포본에서 옮긴다. 서버 수가 바뀌면 실행기를 다시 연다 (서버마다 감시 · 컴파일 줄을 켤 때 만든다).
#[tauri::command]
async fn eclipse_import(app: AppHandle, ws: String, ids: Vec<String>) -> Result<String, String> {
    let picked: Vec<eclipse::EServer> = {
        let w = PathBuf::from(&ws);
        tauri::async_runtime::spawn_blocking(move || eclipse::read_workspace(&w)).await.map_err(|e| e.to_string())?
            .servers.into_iter().filter(|s| ids.contains(&s.id) && !s.project.is_empty()).collect()
    };
    if picked.is_empty() {
        return Err("가져올 서버가 없습니다 (프로젝트 폴더를 찾지 못했습니다).".into());
    }
    for (i, r) in shared(&app).rts.iter().enumerate() {
        if r.inner.lock().unwrap().status != Status::Stopped {
            return Err(format!("{} 톰캣을 끈 뒤에 가져오세요.", cfgs(&app, i).1.name));
        }
    }
    let before = shared(&app).rts.len();
    let mut c = shared(&app).cfg.lock().unwrap().clone();
    let mut jobs = Vec::new();
    for es in &picked {
        let same = |a: &str| a.trim_end_matches('/').eq_ignore_ascii_case(es.project.trim_end_matches('/'));
        let k = match c.servers.iter().position(|s| same(&s.project)).or_else(|| c.servers.iter().position(|s| s.project.trim().is_empty())) {
            Some(k) => k,
            None => {
                let n = c.servers.len();
                let id = (n + 1..).map(|k| format!("app{k}")).find(|id| c.servers.iter().all(|s| &s.id != id)).unwrap_or_default();
                c.servers.push(new_server(&home_dir(), &id, &es.name, n as u16));
                n
            }
        };
        let sv = &mut c.servers[k];
        sv.name = es.name.clone();
        sv.project = es.project.clone();
        if es.http_port > 0 { sv.http_port = es.http_port; }
        if es.ajp_port > 0 { sv.ajp_port = es.ajp_port; }
        if es.shutdown_port > 0 { sv.shutdown_port = es.shutdown_port; }
        for a in &es.jvm_args {
            if !sv.jvm_args.contains(a) { sv.jvm_args.push(a.clone()); }
        }
        if !es.tomcat_home.is_empty() { c.tomcat_home = es.tomcat_home.clone(); }
        if !es.lib_dir.is_empty() { jobs.push((sv.name.clone(), es.lib_dir.clone(), es.project.clone(), PathBuf::from(&sv.deps_dir))); }
    }
    save_config(&c).map_err(|e| format!("설정을 저장하지 못했습니다: {e}"))?;
    let restart = c.servers.len() != before;
    *shared(&app).cfg.lock().unwrap() = c;
    let mut notes = Vec::new();
    for (name, lib, project, deps) in jobs {
        let r = tauri::async_runtime::spawn_blocking(move || eclipse::copy_deps(&lib, &project, &deps)).await.map_err(|e| e.to_string())?;
        match r {
            Ok(n) => {
                log(&app, None, "app", "ok", format!("{name} 의존 jar {n}개를 이클립스 배포본에서 옮겼습니다."));
                notes.push(format!("{name} jar {n}개"));
            }
            Err(e) => {
                log(&app, None, "app", "err", format!("{name} {e}"));
                notes.push(format!("{name} jar 못 옮김"));
            }
        }
    }
    // 옮긴 jar 의 시각을 남겨 「의존 jar 낡음」 이 뜨지 않게 한다
    for sv in shared(&app).cfg.lock().unwrap().servers.iter() {
        if Path::new(&sv.deps_dir).is_dir() && read_stamp(&sv.deps_dir) == 0 {
            let _ = fs::write(Path::new(&sv.deps_dir).join(STAMP_FILE), now_ms().to_string());
        }
    }
    stop_daemon(&app, "");
    let msg = format!("이클립스에서 서버 {}개를 가져왔습니다 · {}", picked.len(), notes.join(" · "));
    log(&app, None, "app", "ok", msg.clone());
    if restart {
        // 서버 수가 바뀌었다 — 새 설정으로 다시 연다
        app.restart();
    }
    emit_state(&app);
    Ok(msg)
}

// 「시작하기 › 프로젝트 폴더 고르기」 — 그 서버의 프로젝트를 정하고, 의존 jar 는 이클립스 배포본이 있으면 옮기고 없으면 Maven 으로 받는다
#[tauri::command]
async fn quick_project(app: AppHandle, srv: usize, project: String) -> Result<String, String> {
    check_srv(&app, srv)?;
    let project = project.trim().replace('\\', "/").trim_end_matches('/').to_string();
    if !Path::new(&project).join("src/main/webapp").is_dir() {
        return Err(format!("src/main/webapp 이 있는 프로젝트 폴더를 고르세요: {project}"));
    }
    let (name, deps) = {
        let mut c = shared(&app).cfg.lock().unwrap();
        let sv = &mut c.servers[srv];
        sv.project = project.clone();
        // 기본 이름(「앱」 · 「앱 2」)이면 폴더 이름으로
        if sv.name.starts_with("앱") {
            sv.name = Path::new(&project).file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
        }
        let out = (sv.name.clone(), PathBuf::from(&sv.deps_dir));
        save_config(&c).map_err(|e| format!("설정을 저장하지 못했습니다: {e}"))?;
        out
    };
    emit_state(&app);
    // 이 프로젝트를 배포한 이클립스 서버가 있으면 그 jar 를 쓴다
    let proj = project.clone();
    let found = tauri::async_runtime::spawn_blocking(move || {
        let parent = Path::new(&proj).parent().map(Path::to_path_buf).into_iter().collect::<Vec<_>>();
        eclipse::find_workspaces(&parent).iter().flat_map(|w| eclipse::read_workspace(w).servers)
            .find(|s| s.project.eq_ignore_ascii_case(&proj) && !s.lib_dir.is_empty())
    }).await.map_err(|e| e.to_string())?;
    if let Some(es) = found {
        let n = tauri::async_runtime::spawn_blocking(move || eclipse::copy_deps(&es.lib_dir, &es.project, &deps)).await.map_err(|e| e.to_string())??;
        let _ = fs::write(Path::new(&cfgs(&app, srv).1.deps_dir).join(STAMP_FILE), now_ms().to_string());
        stop_daemon(&app, "");
        emit_state(&app);
        return Ok(format!("{name} · 의존 jar {n}개를 이클립스 배포본에서 옮겼습니다. 시작을 누르세요."));
    }
    if Path::new(&project).join("pom.xml").is_file() {
        refresh_deps(app.clone(), srv)?;
        return Ok(format!("{name} · 의존 jar 를 Maven 으로 받는 중입니다 (빌드 탭). 끝나면 시작을 누르세요."));
    }
    Ok(format!("{name} · 프로젝트를 정했습니다. 의존 jar 는 src/main/webapp/WEB-INF/lib 의 것만 씁니다."))
}

// 폴더 고르기 창 — 이클립스 작업 공간 · 톰캣 폴더를 고를 때
#[tauri::command]
async fn pick_dir(app: AppHandle, title: String) -> Result<Option<String>, String> {
    use tauri_plugin_dialog::DialogExt;
    tauri::async_runtime::spawn_blocking(move || {
        Ok(app.dialog().file().set_title(title).blocking_pick_folder().and_then(|f| f.into_path().ok()).map(|d| p(&d)))
    })
    .await
    .map_err(|e| e.to_string())?
}

/* ---------- 업데이트 ---------- */

// 확인은 조용히 한다 — 회사망에서 GitHub 에 닿지 않을 수 있으므로 실패는 설정 › 업데이트에만 보이고 알림 줄에 올리지 않는다
fn check_update(app: &AppHandle) {
    {
        let mut u = shared(app).update.lock().unwrap();
        if u.checking || u.downloading.is_some() {
            return;
        }
        u.checking = true;
        u.error = None;
    }
    emit_state(app);
    let r = update::check();
    {
        let mut u = shared(app).update.lock().unwrap();
        u.checking = false;
        u.checked_at_ms = Some(now_ms());
        u.installable = update::installable(&home_dir());
        match r {
            Ok(rel) => {
                if let Some(x) = &rel {
                    log(app, None, "app", "", format!("새 판이 있습니다: {} (지금 {})", x.version, env!("CARGO_PKG_VERSION")));
                }
                u.release = rel;
            }
            Err(e) => u.error = Some(e),
        }
    }
    emit_state(app);
}

// 켠 뒤 30초, 그 뒤 6시간마다 (설정에서 끌 수 있다)
fn update_loop(app: AppHandle) {
    thread::sleep(Duration::from_secs(30));
    loop {
        if app.state::<Shared>().cfg.lock().unwrap().update_check {
            check_update(&app);
        }
        thread::sleep(Duration::from_secs(6 * 3600));
    }
}

// 이 실행 파일과 함께 만든 CHANGELOG.md — 설정 › 업데이트의 「변경 기록」이 그린다 (GitHub 에 닿지 않아도 보인다)
#[tauri::command]
fn changelog() -> &'static str {
    include_str!("../../CHANGELOG.md")
}

#[tauri::command]
fn update_check(app: AppHandle) {
    thread::spawn(move || check_update(&app));
}

// 받아서 해시를 대조한 뒤, 톰캣을 끄고 설치 파일을 조용히 돌리고 실행기를 닫는다. 설치 파일이 끝나면 실행기를 다시 띄운다.
#[tauri::command]
fn update_install(app: AppHandle) -> Result<(), String> {
    let rel = {
        let mut u = shared(&app).update.lock().unwrap();
        if u.downloading.is_some() {
            return Err("이미 받는 중입니다.".into());
        }
        let Some(rel) = u.release.clone() else { return Err("새 판이 없습니다.".into()) };
        if !update::installable(&home_dir()) {
            return Err("설치본이 아니라 여기서 설치할 수 없습니다. 릴리스 페이지에서 받으세요.".into());
        }
        u.downloading = Some(0);
        u.error = None;
        rel
    };
    emit_state(&app);
    thread::spawn(move || {
        log(&app, None, "app", "", format!("업데이트 {} 를 받습니다: {}", rel.version, rel.asset));
        let a = app.clone();
        let r = update::download(&rel, move |got| {
            shared(&a).update.lock().unwrap().downloading = Some(got);
            emit_state(&a);
        });
        match r {
            Ok(setup) => {
                log(&app, None, "app", "", format!("받았습니다 (해시 일치). 톰캣을 끄고 {} 를 설치합니다.", rel.version));
                quit_then(app, move |app| {
                    if let Err(e) = update::run_installer(&setup, &home_dir()) {
                        log(app, None, "app", "err", e);
                    }
                });
            }
            Err(e) => {
                log(&app, None, "app", "err", e.clone());
                let mut u = shared(&app).update.lock().unwrap();
                u.downloading = None;
                u.error = Some(e);
                drop(u);
                emit_state(&app);
            }
        }
    });
    Ok(())
}

#[tauri::command]
fn open(target: String) {
    // 주소든 폴더든 탐색기에 맡기면 기본 브라우저 · 탐색기로 열린다
    let arg = if target.starts_with("http") { target } else { target.replace('/', "\\") };
    let _ = Command::new("explorer").arg(arg).spawn();
}

#[tauri::command]
fn reveal(path: String) {
    // 파일을 고른 채 그 폴더를 연다 (우클릭 「탐색기에서 보기」). 지운 파일처럼 없으면 남아 있는 가장 가까운 부모 폴더를 연다
    let win = path.replace('/', "\\");
    let p = Path::new(&win);
    if p.exists() {
        // 「/select,"경로"」 는 한 덩어리로 넘긴다. arg 로 넘기면 공백 든 경로를 통째로 따옴표로 감싸 explorer 가 /select 를 읽지 못하고 문서 폴더를 연다
        let _ = Command::new("explorer").raw_arg(format!("/select,\"{win}\"")).spawn();
    } else if let Some(dir) = p.ancestors().skip(1).find(|d| d.is_dir()) {
        let _ = Command::new("explorer").arg(dir).spawn();
    }
}

// 읽기 전용 SVN — 상태 · 차이. 1~2초 걸리므로 화면 스레드가 아니라 따로 돌린다.
#[tauri::command]
async fn svn_status(app: AppHandle, srv: usize) -> Result<svn::Status, String> {
    check_srv(&app, srv)?;
    let wc = PathBuf::from(vcs_root(&cfgs(&app, srv).1));
    tauri::async_runtime::spawn_blocking(move || svn::status(&wc)).await.map_err(|e| e.to_string())?
}

#[tauri::command]
async fn svn_diff(app: AppHandle, srv: usize, path: String) -> Result<svn::Diff, String> {
    check_srv(&app, srv)?;
    if path.contains("..") {
        return Err("작업복사본 밖 경로입니다.".into());
    }
    let wc = PathBuf::from(vcs_root(&cfgs(&app, srv).1));
    tauri::async_runtime::spawn_blocking(move || svn::diff(&wc, &path)).await.map_err(|e| e.to_string())?
}

/* ---------- 저장소 조회 (읽기 전용 · 업데이트 확인 · 로그 · 리비전 차이) ---------- */

// 화면이 이 앞머리를 보고 로그인 칸을 띄운다
const NEED_LOGIN: &str = "NEED_LOGIN|";

fn web_err(e: svnweb::WebErr) -> String {
    match e {
        svnweb::WebErr::NeedLogin(m) => format!("{NEED_LOGIN}{m}"),
        other => other.to_string(),
    }
}

fn wc_of(app: &AppHandle, srv: usize) -> Result<(PathBuf, svn::WcInfo), String> {
    check_srv(app, srv)?;
    let wc = PathBuf::from(vcs_root(&cfgs(app, srv).1));
    let info = svn::wc_info(&wc)?;
    Ok((wc, info))
}

// 이번 실행에서 넣은 계정이 먼저, 없으면 자격 증명 관리자
fn creds_for(app: &AppHandle, url: &str) -> Result<(String, String), String> {
    let host = svnweb::host_of(url);
    if let Some(c) = shared(app).svn_creds.lock().unwrap().get(&host) {
        return Ok(c.clone());
    }
    svnweb::cred_get(&host).ok_or_else(|| format!("{NEED_LOGIN}{host} 저장소에 로그인해야 합니다."))
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Account {
    host: String,
    user: Option<String>,
    saved: bool,
}

#[tauri::command]
async fn svn_account(app: AppHandle, srv: usize) -> Result<Account, String> {
    let (_, info) = wc_of(&app, srv)?;
    let host = svnweb::host_of(&info.url);
    if let Some((u, _)) = shared(&app).svn_creds.lock().unwrap().get(&host) {
        return Ok(Account { host, user: Some(u.clone()), saved: false });
    }
    let saved = svnweb::cred_get(&host).map(|(u, _)| u);
    Ok(Account { host, saved: saved.is_some(), user: saved })
}

// 계정을 저장소에 한 번 물어 맞는지 본 뒤에만 넣어 둔다. 비밀번호는 로그 · 화면 어디에도 내지 않는다.
#[tauri::command]
async fn svn_login(app: AppHandle, srv: usize, user: String, pass: String, save: bool) -> Result<svnweb::Head, String> {
    let (_, info) = wc_of(&app, srv)?;
    let host = svnweb::host_of(&info.url);
    let cred = (user.trim().to_string(), pass);
    let url = info.url.clone();
    let c2 = cred.clone();
    let head = tauri::async_runtime::spawn_blocking(move || svnweb::options(&url, &c2)).await.map_err(|e| e.to_string())?.map_err(web_err)?;
    if save {
        svnweb::cred_set(&host, &cred.0, &cred.1)?;
        shared(&app).svn_creds.lock().unwrap().remove(&host);
    } else {
        shared(&app).svn_creds.lock().unwrap().insert(host.clone(), cred.clone());
    }
    log(&app, None, "app", "", format!("SVN 계정을 확인했습니다 ({} · {host}{}).", cred.0, if save { " · 자격 증명 관리자에 저장" } else { " · 이번 실행에서만" }));
    Ok(head)
}

#[tauri::command]
async fn svn_logout(app: AppHandle, srv: usize) -> Result<(), String> {
    let (_, info) = wc_of(&app, srv)?;
    let host = svnweb::host_of(&info.url);
    shared(&app).svn_creds.lock().unwrap().remove(&host);
    svnweb::cred_delete(&host);
    Ok(())
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Incoming {
    head: i64,
    base: i64,             // 작업복사본에서 가장 높은 리비전
    entries: Vec<svnweb::LogEntry>,
    overlap: Vec<String>,  // 들어올 변경과 내가 고친 파일이 겹치는 경로 — 업데이트하면 충돌할 수 있다
    more: bool,            // 너무 많아 앞의 200개만 가져왔다
}

// 업데이트 확인 — 저장소 최신 리비전과, 작업복사본 다음부터 최신까지의 커밋 (svn log · 읽기 전용)
#[tauri::command]
async fn svn_incoming(app: AppHandle, srv: usize) -> Result<Incoming, String> {
    let (wc, info) = wc_of(&app, srv)?;
    let cred = creds_for(&app, &info.url)?;
    tauri::async_runtime::spawn_blocking(move || -> Result<Incoming, String> {
        let head = svnweb::options(&info.url, &cred).map_err(web_err)?;
        let base = info.rev_max;
        let mut entries = Vec::new();
        if head.head > base {
            entries = svnweb::log(&info.url, &info.repos_path, &cred, head.head, base + 1, 200, "").map_err(web_err)?;
        }
        let more = entries.len() >= 200;
        let mine: HashSet<String> =
            svn::status(&wc).map(|st| st.items.into_iter().filter(|i| i.code != "?").map(|i| i.path).collect()).unwrap_or_default();
        let mut overlap: Vec<String> = entries.iter().flat_map(|e| e.paths.iter()).filter(|c| c.inside && mine.contains(&c.path)).map(|c| c.path.clone()).collect();
        overlap.sort();
        overlap.dedup();
        Ok(Incoming { head: head.head, base, entries, overlap, more })
    })
    .await
    .map_err(|e| e.to_string())?
}

// 로그 보기 — 최신부터 limit 개 (rel 이 비면 작업복사본 전체, 아니면 그 파일 · 폴더)
#[tauri::command]
async fn svn_log(app: AppHandle, srv: usize, rel: String, limit: u32) -> Result<Vec<svnweb::LogEntry>, String> {
    if rel.contains("..") {
        return Err("작업복사본 밖 경로입니다.".into());
    }
    let (_, info) = wc_of(&app, srv)?;
    let cred = creds_for(&app, &info.url)?;
    tauri::async_runtime::spawn_blocking(move || {
        let head = svnweb::options(&info.url, &cred).map_err(web_err)?;
        svnweb::log(&info.url, &info.repos_path, &cred, head.head, 1, limit.clamp(1, 500), &rel).map_err(web_err)
    })
    .await
    .map_err(|e| e.to_string())?
}

// 두 내용의 차이. 쪽마다 「local」(지금 파일) · 「base」(기준본) · 「head」(저장소 최신) · 리비전 번호 중 하나
#[tauri::command]
async fn svn_rev_diff(app: AppHandle, srv: usize, rel: String, old: String, new: String) -> Result<svn::Diff, String> {
    if rel.contains("..") || rel.is_empty() {
        return Err("파일 경로가 올바르지 않습니다.".into());
    }
    let (wc, info) = wc_of(&app, srv)?;
    let needs_server = [&old, &new].iter().any(|s| s.as_str() != "local" && s.as_str() != "base");
    let cred = if needs_server { Some(creds_for(&app, &info.url)?) } else { None };
    tauri::async_runtime::spawn_blocking(move || -> Result<svn::Diff, String> {
        let mut head: Option<svnweb::Head> = None;
        let mut get = |spec: &str| -> Result<Vec<u8>, String> {
            match spec {
                "local" => Ok(fs::read(wc.join(&rel)).unwrap_or_default()),
                "base" => Ok(svn::base_content(&wc, &rel).unwrap_or_default()),
                _ => {
                    let cred = cred.as_ref().ok_or("SVN 계정이 필요합니다.")?;
                    if head.is_none() {
                        head = Some(svnweb::options(&info.url, cred).map_err(web_err)?);
                    }
                    let h = head.as_ref().unwrap();
                    let rev = if spec == "head" { h.head } else { spec.parse().map_err(|_| format!("리비전이 올바르지 않습니다: {spec}"))? };
                    let path = format!("{}/{}", info.repos_path, rel);
                    // 그 리비전에 없던 파일(추가 · 삭제 전후)은 빈 내용과 견준다
                    Ok(svnweb::cat(&info.repo_root, &h.rev_root_stub, rev, &path, cred).map_err(web_err)?.unwrap_or_default())
                }
            }
        };
        let (a, b) = (get(&old)?, get(&new)?);
        Ok(svn::text_diff(&rel, &a, &b))
    })
    .await
    .map_err(|e| e.to_string())?
}

/* ---------- 읽기 전용 Git (내 변경 · 받을 커밋 · 로그 · 커밋 차이) ---------- */

fn git_root(app: &AppHandle, srv: usize) -> Result<PathBuf, String> {
    check_srv(app, srv)?;
    Ok(PathBuf::from(vcs_root(&cfgs(app, srv).1)))
}

fn safe_rel(rel: &str) -> Result<(), String> {
    if rel.contains("..") || rel.starts_with('-') {
        return Err("저장소 밖 경로입니다.".into());
    }
    Ok(())
}

// git 명령은 저장소 크기에 따라 1초 넘게 걸리므로 화면 스레드가 아니라 따로 돌린다
#[tauri::command]
async fn git_status(app: AppHandle, srv: usize) -> Result<git::Status, String> {
    let root = git_root(&app, srv)?;
    tauri::async_runtime::spawn_blocking(move || git::status(&root)).await.map_err(|e| e.to_string())?
}

#[tauri::command]
async fn git_diff(app: AppHandle, srv: usize, path: String, orig: Option<String>) -> Result<svn::Diff, String> {
    safe_rel(&path)?;
    if let Some(o) = &orig {
        safe_rel(o)?;
    }
    let root = git_root(&app, srv)?;
    tauri::async_runtime::spawn_blocking(move || git::diff(&root, &path, orig.as_deref())).await.map_err(|e| e.to_string())?
}

#[tauri::command]
async fn git_incoming(app: AppHandle, srv: usize) -> Result<git::Incoming, String> {
    let root = git_root(&app, srv)?;
    tauri::async_runtime::spawn_blocking(move || git::incoming(&root)).await.map_err(|e| e.to_string())?
}

#[tauri::command]
async fn git_log(app: AppHandle, srv: usize, rel: String, limit: u32) -> Result<Vec<git::Commit>, String> {
    safe_rel(&rel)?;
    let root = git_root(&app, srv)?;
    tauri::async_runtime::spawn_blocking(move || git::log(&root, &rel, limit.clamp(1, 500))).await.map_err(|e| e.to_string())?
}

#[tauri::command]
async fn git_rev_diff(app: AppHandle, srv: usize, rel: String, old: String, new: String) -> Result<svn::Diff, String> {
    safe_rel(&rel)?;
    if rel.is_empty() {
        return Err("파일 경로가 올바르지 않습니다.".into());
    }
    let root = git_root(&app, srv)?;
    tauri::async_runtime::spawn_blocking(move || git::rev_diff(&root, &rel, &old, &new)).await.map_err(|e| e.to_string())?
}

// 「설정 › 버전 관리」 — 바로 적용된다 (조회는 부를 때마다 설정을 읽는다)
#[tauri::command]
fn set_vcs(app: AppHandle, srv: usize, vcs: VcsCfg) -> Result<(), String> {
    check_srv(&app, srv)?;
    if !VCS_KINDS.contains(&vcs.kind.as_str()) {
        return Err(format!("버전 관리 종류가 올바르지 않습니다: {}", vcs.kind));
    }
    let root = vcs.root.trim();
    if !root.is_empty() && !Path::new(root).is_dir() {
        return Err(format!("작업 폴더가 없습니다: {root}"));
    }
    let c = {
        let mut c = shared(&app).cfg.lock().unwrap();
        c.servers[srv].vcs = VcsCfg {
            kind: vcs.kind,
            root: root.replace('\\', "/"),
            commit_hint: vcs.commit_hint.trim().to_string(),
            svn_update: vcs.svn_update.trim().to_string(),
            git_update: vcs.git_update.trim().to_string(),
        };
        c.clone()
    };
    save_config(&c).map_err(|e| format!("설정 파일을 쓰지 못했습니다: {e}"))?;
    emit_state(&app);
    let _ = app.emit("vcs-dirty", srv);
    Ok(())
}

#[tauri::command]
fn open_source(app: AppHandle, file: String, line: u32) {
    let editor = app.state::<Shared>().cfg.lock().unwrap().editor.clone();
    if editor.is_empty() {
        let _ = Command::new("explorer").arg(file.replace('/', "\\")).spawn();
        return;
    }
    // code 는 code.cmd 라 cmd 를 거친다. 「파일:줄」 로 그 줄에서 연다
    let r = Command::new("cmd")
        .args(["/c", &editor, "-g", &format!("{file}:{line}")])
        .creation_flags(CREATE_NO_WINDOW)
        .spawn();
    if let Err(e) = r {
        log(&app, None, "app", "err", format!("편집기를 열지 못했습니다 ({editor}): {e}"));
    }
}

fn main() {
    let (cfg, cfg_note) = load_config();
    let (log_tx, log_rx) = mpsc::channel();
    let (change_tx, change_rx) = mpsc::channel();
    let mut compile_rxs = Vec::new();
    let rts: Vec<Rt> = cfg
        .servers
        .iter()
        .map(|_| {
            let (tx, rx) = mpsc::channel();
            compile_rxs.push(rx);
            Rt {
                inner: Mutex::new(Inner {
                    child: None,
                    status: Status::Stopped,
                    started_at_ms: None,
                    startup_ms: None,
                    spawn_ms: 0,
                    marks: Vec::new(),
                    phases: Vec::new(),
                    mode: String::new(),
                    java: String::new(),
                    waiting: false,
                    generation: 0,
                    mapper_hotswap: false,
                    mapper_wait: None,
                    session_had: None,
                    session_err: None,
                    mem: Mem::default(),
                    compiling: false,
                    last_compile: None,
                    restart_needed: Vec::new(),
                }),
                compile_lock: Mutex::new(()),
                compile_tx: tx,
                console: Mutex::new(None),
                deps_busy: AtomicBool::new(false),
                live_mappers: Mutex::new(Vec::new()),
                svn_stamp: AtomicU64::new(0),
                git_stamp: AtomicU64::new(0),
            }
        })
        .collect();
    let shared = Shared {
        auto_compile: AtomicBool::new(cfg.auto_compile),
        cfg: Mutex::new(cfg),
        rts,
        log_tx,
        change_tx,
        watchers: Mutex::new(Vec::new()),
        daemon: Mutex::new(None),
        daemon_alive: AtomicBool::new(false),
        daemon_last_use: AtomicU64::new(0),
        daemon_kind: Mutex::new(String::new()),
        daemon_mem: Mutex::new(Mem::default()),
        launcher_mem: Mutex::new(Mem::default()),
        start_gate: Mutex::new(()),
        svn_creds: Mutex::new(std::collections::HashMap::new()),
        job: make_job(),
        quitting: AtomicBool::new(false),
        update: Mutex::new(UpdateState::default()),
    };

    tauri::Builder::default()
        // 두 번 띄우면 두 실행기가 같은 포트 · 폴더를 두고 다툰다. 이미 떠 있으면 그 창을 앞으로 가져온다.
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            if let Some(w) = app.get_webview_window("main") {
                let _ = w.unminimize();
                let _ = w.set_focus();
            }
        }))
        // 폴더 고르기 창 — 실행기 쪽(pick_project)에서만 부르므로 화면 권한(capabilities)은 더하지 않는다
        .plugin(tauri_plugin_dialog::init())
        .manage(shared)
        .setup(move |app| {
            let h = app.handle().clone();
            let h2 = h.clone();
            thread::spawn(move || log_pump(h2, log_rx));
            for (i, rx) in compile_rxs.into_iter().enumerate() {
                let hc = h.clone();
                thread::spawn(move || compile_worker(hc, i, rx));
            }
            let h4 = h.clone();
            thread::spawn(move || change_worker(h4, change_rx));
            if let Some(note) = &cfg_note {
                log(&h, None, "app", "warn", note.clone());
            }
            log(&h, None, "app", "", format!("설정 파일: {}", p(&config_path())));
            start_watchers(&h);
            let h5 = h.clone();
            thread::spawn(move || warm_daemon(&h5));
            let h7 = h.clone();
            thread::spawn(move || update_loop(h7));
            let h6 = h.clone();
            thread::spawn(move || monitor(h6));
            Ok(())
        })
        .on_window_event(|window, event| {
            if let WindowEvent::CloseRequested { api, .. } = event {
                // 2026-10-28 : 켜진 톰캣이 있으면 바로 끄지 않고 화면에 묻는다. 끄는 몇 초 동안 창이 그대로라 닫기가 안 먹은 것처럼 보였다.
                // 제목 줄 단추 · Alt+F4 · 작업 표시줄 모두 여기로 온다. 확인하면 화면이 `quit` 를 부른다.
                let app = window.app_handle().clone();
                if app.state::<Shared>().quitting.load(Ordering::Relaxed) {
                    api.prevent_close();
                } else {
                    let running = running_names(&app);
                    if !running.is_empty() {
                        api.prevent_close();
                        let _ = app.emit("close-ask", running);
                    }
                }
            }
        })
        .invoke_handler(tauri::generate_handler![
            get_state, start, stop, restart, quit, compile, refresh_deps, set_option, set_heap, set_compiler, set_ignore, open, reveal, open_source,
            svn_status, svn_diff, svn_account, svn_login, svn_logout, svn_incoming, svn_log, svn_rev_diff,
            git_status, git_diff, git_incoming, git_log, git_rev_diff, set_vcs, set_paths, set_server, add_server, pick_project,
            update_check, update_install, changelog, eclipse_scan, eclipse_import, quick_project, pick_dir
        ])
        .run(tauri::generate_context!())
        .expect("실행기를 띄우지 못했습니다");
}

#[cfg(test)]
mod tests {
    use super::*;

    const PROJ: &str = "D:/work/shop";

    // 프로젝트의 context-datasource.xml 을 본뜬 것 — 속성(p:mapperLocations) · 목록(<list><value>) 두 꼴,
    // 주석으로 막은 빈, 다시 읽지 않는 일반 빈(SqlSessionFactoryBean)이 섞여 있다.
    const DATASOURCE_XML: &str = r#"<beans>
  <!-- <bean id="oldSession" class="org.mybatis.spring.RefreshableSqlSessionFactoryBean"
      p:mapperLocations="classpath*:/sqlmap/old/**/*.xml"/> -->
  <bean id="mainSession" class="org.mybatis.spring.RefreshableSqlSessionFactoryBean"
      p:dataSource-ref="ds" p:configLocation="classpath:/sqlmap/sql-mapper-config.xml"
      p:mapperLocations="classpath*:/sqlmap/main/**/**/*.xml"/>
  <bean id="subSession" class="org.mybatis.spring.RefreshableSqlSessionFactoryBean">
    <property name="dataSource" ref="ds2"/>
    <property name="mapperLocations">
      <list>
        <value>classpath*:/sqlmap/sub/**/**/*.xml</value>
        <value>classpath*:/sqlmap/geo/gis/GisMapper.xml</value>
      </list>
    </property>
  </bean>
  <bean id="batchSession" class="org.mybatis.spring.SqlSessionFactoryBean"
      p:mapperLocations="classpath*:/sqlmap/batch/**/*.xml"/>
</beans>"#;

    fn live() -> Vec<String> {
        parse_live_mappers(DATASOURCE_XML)
    }

    fn kind(path: &str) -> String {
        match classify_path(PROJ, &live(), Path::new(path)) {
            Some(Change::Java) => "java".into(),
            Some(Change::Pom) => "pom".into(),
            Some(Change::Live(what, _)) => format!("live:{what}"),
            Some(Change::Restart(_, why)) => format!("restart:{why}"),
            None => "-".into(),
        }
    }

    #[test]
    fn 저장한_파일을_반영_방식으로_나눈다() {
        // 감시기는 역슬래시 · 다른 대소문자로 경로를 줄 수 있다
        assert_eq!(kind(r"d:\Work\shop\src\main\java\com\example\web\OrderController.java"), "java");
        assert_eq!(kind("D:/work/shop/src/main/webapp/js/order/order.js"), "live:화면");
        assert_eq!(kind("D:/work/shop/src/main/webapp/WEB-INF/views/order/orderList.jsp"), "live:화면");
        assert_eq!(kind("D:/work/shop/src/main/webapp/WEB-INF/views/order/orderPart.jspf"), "live:화면");
        assert_eq!(kind("D:/work/shop/src/main/webapp/WEB-INF/views/order/orderList.jsp.tmp.23320.f28702042099"), "-");
        assert_eq!(kind("D:/work/shop/src/main/webapp/WEB-INF/web.xml"), "restart:WEB-INF 설정");
        assert_eq!(kind("D:/work/shop/src/main/webapp/WEB-INF/lib/a.jar"), "restart:jar");
        assert_eq!(kind("D:/work/shop/src/main/resources/sqlmap/main/order/OrderMapper.xml"), "live:매퍼");
        assert_eq!(kind("D:/work/shop/src/main/resources/sqlmap/sub/a/B.xml"), "live:매퍼");
        assert_eq!(kind("D:/work/shop/src/main/resources/sqlmap/subx/a/B.xml"), "restart:자동으로 다시 읽지 않는 매퍼");
        assert_eq!(kind("D:/work/shop/src/main/resources/sqlmap/batch/a/B.xml"), "restart:자동으로 다시 읽지 않는 매퍼");
        assert_eq!(kind("D:/work/shop/src/main/resources/sqlmap/geo/gis/gisMapper.xml"), "live:매퍼");
        assert_eq!(kind("D:/work/shop/src/main/resources/spring/context-datasource.xml"), "restart:Spring 설정");
        assert_eq!(kind("D:/work/shop/src/main/resources/message/message_ko.properties"), "restart:메시지 (캐시를 비우지 않음)");
        assert_eq!(kind("D:/work/shop/pom.xml"), "pom");
        // 로컬 실행에 쓰지 않는 것 · 편집기 임시 파일 · 폴더 · 다른 프로젝트
        assert_eq!(kind("D:/work/shop/src/main/resources/resources-dev/spring/a.xml"), "-");
        assert_eq!(kind("D:/work/shop/src/main/java/com/example/X.java~"), "-");
        assert_eq!(kind("D:/work/shop/src/main/webapp/js/order"), "-");
        assert_eq!(kind("D:/work/shop/.classpath"), "-");
        assert_eq!(kind("D:/work/shopper/src/main/java/X.java"), "-");
        assert_eq!(kind("D:/work/admin/src/main/java/X.java"), "-");
    }

    #[test]
    fn 무시_목록을_폴더_이름_패턴으로_맞춘다() {
        let pats = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<String>>();
        // 폴더 아래 전부 (끝의 「/」 · 대소문자 · 역슬래시와 상관없이)
        let dep = pats(&["src/main/resources/db/deploy/"]);
        assert!(ignored(&dep, "src/main/resources/db/deploy/15_accounts.sql"));
        assert!(ignored(&dep, r"src\main\resources\DB\deploy\sub\a.sql"));
        assert!(!ignored(&dep, "src/main/resources/db/deployer.sql"));
        assert!(!ignored(&dep, "src/main/resources/sqlmap/main/order/OrderMapper.xml"));
        // 「/」 없는 패턴은 어느 단계에서나
        assert!(ignored(&pats(&["*.sql"]), "src/main/resources/a/b.sql"));
        assert!(!ignored(&pats(&["*.sql"]), "src/main/resources/a/b.sql.bak"));
        assert!(ignored(&pats(&["deploy/"]), "src/main/resources/db/deploy/x.sql"));
        assert!(ignored(&pats(&["order*.tmp"]), "src/main/webapp/orderList.tmp"));
        // 「/」 있는 패턴은 맨 위부터 · 「*」 는 한 단계 · 「**」 는 여러 단계
        assert!(ignored(&pats(&["src/main/webapp/docs"]), "src/main/webapp/docs/order/review.html"));
        assert!(!ignored(&pats(&["webapp/docs"]), "src/main/webapp/docs/order/review.html"));
        assert!(ignored(&pats(&["src/main/*/db"]), "src/main/resources/db/deploy/a.sql"));
        assert!(!ignored(&pats(&["src/*/db"]), "src/main/resources/db/deploy/a.sql"));
        assert!(ignored(&pats(&["src/**/deploy"]), "src/main/resources/db/deploy/a.sql"));
        assert!(ignored(&pats(&["src/**/a.sql"]), "src/a.sql"));
        assert!(ignored(&pats(&["./src/main/resources/rules/"]), "src/main/resources/rules/rules_v2.json"));
        // 빈 줄은 아무것도 막지 않는다 · 기본값은 비어 있다
        assert!(!ignored(&pats(&["", " / "]), "src/main/java/X.java"));
        assert!(Config::default().ignore.is_empty());
        // 파일 이름만 적으면 어느 폴더의 그 파일이든 (이름이 통째로 같을 때만)
        assert!(ignored(&pats(&["a.sql"]), "src/main/resources/db/a.sql"));
        assert!(ignored(&pats(&["A.SQL"]), "a.sql"));
        assert!(!ignored(&pats(&["a.sql"]), "src/main/resources/db/xa.sql"));
        // 파일 경로는 그 파일 하나만
        assert!(ignored(&pats(&["src/main/resources/db/a.sql"]), "src/main/resources/db/a.sql"));
        assert!(!ignored(&pats(&["src/main/resources/db/a.sql"]), "src/main/resources/b/a.sql"));
    }

    #[test]
    fn 무시_목록을_저장할_꼴로_다듬는다() {
        let projects = vec![r"D:\work\shop\".to_string(), "D:/work/admin".to_string()];
        let list = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<String>>();
        // 탐색기 「경로로 복사」 꼴(따옴표 · 절대 경로 · 역슬래시) → 프로젝트 기준
        assert_eq!(
            clean_ignore(&projects, list(&[r#""D:\Work\SHOP\src\main\resources\db\a.sql""#, "d:/work/admin/src/x/"])),
            ["src/main/resources/db/a.sql", "src/x/"]
        );
        // 빈 줄 · 대소문자 · 역슬래시 · 끝의 「/」 만 다른 것은 하나로 (먼저 넣은 꼴을 남긴다)
        assert_eq!(clean_ignore(&projects, list(&["*.sql", "", "  ", "*.SQL", "deploy/", "deploy", r"src\a", "./src/a/"])), ["*.sql", "deploy/", "src/a"]);
        // 다른 폴더의 절대 경로는 그대로 둔다 (프로젝트 이름이 앞부분만 같은 것도)
        assert_eq!(clean_ignore(&projects, list(&["D:/work/shopper/a.sql"])), ["D:/work/shopper/a.sql"]);
    }

    #[test]
    fn 스스로_다시_읽는_매퍼를_설정에서_찾는다() {
        // 주석으로 막은 빈 · 일반 SqlSessionFactoryBean · configLocation 은 넣지 않는다
        assert_eq!(live(), ["sqlmap/main/", "sqlmap/sub/", "sqlmap/geo/gis/gismapper.xml"]);
    }

    #[test]
    fn 설정_파일을_읽고_빠진_옵션은_기본값으로() {
        let c = parse_config(r#"{"hotReload":false,"editor":"notepad","servers":[{"id":"a","project":"D:/x/a","httpPort":1234}]}"#).unwrap();
        assert_eq!(c.servers.len(), 1);
        assert_eq!((c.servers[0].project.as_str(), c.servers[0].http_port), ("D:/x/a", 1234));
        assert!(!c.hot_reload);
        assert_eq!(c.editor, "notepad");
        // 파일에 없는 옵션은 기본값 — 힙 상한 1.5GB · ECJ · 로그인 유지(톰캣 기본) · 앱 동작을 바꾸는 옵션은 꺼짐
        assert_eq!((c.heap_mb, c.compiler.as_str(), c.keep_session, c.stagger_start), (1536, "ecj", true, true));
        assert!(!c.lazy_init && !c.no_schedule && !c.mapper_hotswap && !c.skip_scan);
        // 기본 설정은 빈 서버 하나 — 프로젝트 · 접속 주소를 코드에 두지 않는다
        let def = Config::default();
        assert_eq!(def.servers.len(), 1);
        assert!(def.servers[0].project.is_empty());
        // 열기 단추는 「/」 하나 — 화면이 그 서버의 HTTP 포트를 붙여 연다
        assert_eq!(def.servers[0].links[0].url, "/");
        // 2026-10-28 에 지운 「SVN 로그인 안내」(vcs.accountNote)가 남은 파일도 읽히고, 다시 쓰면 빠진다
        let c = parse_config(r#"{"servers":[{"vcs":{"kind":"svn","accountNote":"x"}}]}"#).unwrap();
        assert_eq!(c.servers[0].vcs.kind, "svn");
        assert!(!serde_json::to_string(&c).unwrap().contains("accountNote"));
    }

    #[test]
    fn 더한_서버는_포트와_폴더가_겹치지_않는다() {
        let h = Path::new("C:/t");
        let a = new_server(h, "app", "앱", 0);
        let b = new_server(h, "app2", "앱 2", 1);
        assert_eq!(a.catalina_base, "C:/t/base");
        assert_eq!(b.catalina_base, "C:/t/servers/app2/base");
        let pa = [a.http_port, a.ajp_port, a.shutdown_port, a.debug_port];
        let pb = [b.http_port, b.ajp_port, b.shutdown_port, b.debug_port];
        assert!(pb.iter().all(|x| !pa.contains(x)), "{pa:?} {pb:?}");
    }

    #[test]
    fn 고른_폴더나_한_단계_아래에서_프로젝트를_찾는다() {
        let t = std::env::temp_dir().join(format!("tomcatrun-pick-{}", std::process::id()));
        let _ = fs::remove_dir_all(&t);
        for d in ["a/src/main/webapp", "b/src/main/webapp", "c/x", "d/e/src/main/webapp"] {
            fs::create_dir_all(t.join(d)).unwrap();
        }
        // 그 폴더 자신이면 그것 하나
        assert_eq!(webapp_roots(&t.join("a")), [p(&t.join("a"))]);
        // 바로 아래 한 단계만 본다 — d/e 는 두 단계라 찾지 않는다
        assert_eq!(webapp_roots(&t), [p(&t.join("a")), p(&t.join("b"))]);
        assert!(webapp_roots(&t.join("c")).is_empty());
        let _ = fs::remove_dir_all(&t);
    }

    #[test]
    fn 비운_경로는_실행기_폴더에서_채우고_저장할_때_뺀다() {
        let def = Config::default();
        let home = p(&home_dir());
        // 경로 칸이 없거나 빈 설정 — 첫 서버는 실행기 폴더의 base, 그 밖은 servers/<id>/
        let text = r#"{"javaHot":"","servers":[{"id":"a","project":"D:/x/a"},{"id":"b","catalinaBase":"E:/my/base"},{"id":"c","depsDir":"out/deps"}]}"#;
        let c = parse_config(text).unwrap();
        assert_eq!(c.java_hot, def.java_hot);
        assert_eq!(c.javac, def.javac);
        assert_eq!(c.servers[0].catalina_base, def.servers[0].catalina_base);
        assert_eq!(c.servers[1].catalina_base, "E:/my/base", "적어 둔 경로는 그대로");
        assert_eq!(c.servers[1].classes_dir, format!("{home}/servers/b/classes"));
        assert_eq!(c.servers[2].deps_dir, format!("{home}/out/deps"), "상대 경로는 실행기 폴더 기준");
        // 저장할 때는 기본값과 같은 칸을 비우고 (역슬래시 · 대소문자 · 끝 빗금은 가리지 않는다), 실행기 폴더 안은 상대 경로로
        let mut c2 = c.clone();
        c2.servers[0].classes_dir = c2.servers[0].classes_dir.replace('/', "\\").to_uppercase() + "\\";
        let s = strip_paths(&c2, &def);
        assert_eq!((s.java_hot.as_str(), s.maven.as_str(), s.javac.as_str()), ("", "", ""));
        assert_eq!((s.servers[0].catalina_base.as_str(), s.servers[0].classes_dir.as_str()), ("", ""));
        assert_eq!(s.servers[1].catalina_base, "E:/my/base");
        assert_eq!(s.servers[2].deps_dir, "out/deps");
        assert_eq!(c2.servers[0].catalina_base, def.servers[0].catalina_base, "실행 중인 설정은 그대로");
        // 비운 채 저장한 파일을 다시 읽으면 같은 경로가 된다
        let back = parse_config(&serde_json::to_string(&s).unwrap()).unwrap();
        assert_eq!(back.servers[0].deps_dir, def.servers[0].deps_dir);
        assert_eq!(back.servers[2].deps_dir, c.servers[2].deps_dir);
        assert_eq!(back.hotswap_agent, def.hotswap_agent);
    }

    #[test]
    fn 톰캣에는_설정의_경로와_포트를_속성으로_넘긴다() {
        let mut cfg = Config::default();
        let mut sv = new_server(Path::new("C:/t"), "app", "앱", 0);
        sv.project = r"D:\work\shop\".into();
        let props = server_props(&cfg, &sv);
        assert!(props.contains(&"-Dtomcatrun.webapp=D:/work/shop/src/main/webapp".to_string()));
        assert!(props.contains(&"-Dtomcatrun.httpPort=8080".to_string()));
        assert!(props.contains(&"-Dtomcatrun.tldScanAll=true".to_string()));
        assert!(props.contains(&"-Dtomcatrun.sciFilter=".to_string()));
        cfg.skip_scan = true;
        cfg.tld_scan = "jstl-*.jar".into();
        let props = server_props(&cfg, &sv);
        assert!(props.contains(&"-Dtomcatrun.tldScanAll=false".to_string()));
        assert!(props.contains(&"-Dtomcatrun.pluggabilityScan=false".to_string()));
        assert!(props.iter().any(|x| x.starts_with("-Dtomcatrun.sciFilter=org")));
        // server.xml 이 쓰는 속성은 모두 넘긴다
        let xml = include_str!("../../base/conf/server.xml");
        for name in xml.split("${tomcatrun.").skip(1).map(|r| &r[..r.find('}').unwrap()]) {
            assert!(props.iter().any(|x| x.starts_with(&format!("-Dtomcatrun.{name}="))), "{name}");
        }
    }

    #[test]
    fn 세션_오류는_예외_줄의_첫_줄로_잡는다() {
        assert_eq!(
            exception_line("java.io.InvalidClassException: com.example.model.UserVO; local class incompatible"),
            Some("java.io.InvalidClassException: com.example.model.UserVO; local class incompatible")
        );
        assert!(exception_line("\tat java.io.ObjectInputStream.readObject(ObjectInputStream.java:431)").is_none());
        assert!(exception_line("Caused by: java.lang.ClassNotFoundException: a.B").is_some());
        assert!(exception_line("28-Oct-2026 11:04:20.606 정보 [main] org.apache.catalina.core.StandardServer.await").is_none());
        assert_eq!(session_file("C:/t/base").as_path(), Path::new("C:/t/base/work/Catalina/localhost/ROOT/SESSIONS.ser"));
        assert_eq!((kb(1), kb(2048)), ("1 KB".to_string(), "2 KB".to_string()));
    }

    #[test]
    fn web_xml_사본은_두_군데만_더한다() {
        // 서블릿 2.5 web.xml (LF) · 이미 metadata-complete 가 있는 3.0 web.xml (CRLF)
        let originals: [&[u8]; 2] = [
            b"<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<web-app xmlns=\"http://java.sun.com/xml/ns/javaee\" version=\"2.5\">\n  <display-name>\xec\x95\xb1</display-name>\n</web-app>\n",
            b"<?xml version=\"1.0\"?>\r\n<web-app version=\"3.0\"\r\n    metadata-complete=\"true\">\r\n  <servlet/>\r\n</web-app>\r\n",
        ];
        for src in originals {
            // 끄면 원본 그대로
            assert_eq!(dev_web_xml(src, false).unwrap(), src);
            let out = dev_web_xml(src, true).unwrap();
            let s = String::from_utf8(out.clone()).expect("UTF-8 그대로");
            let head = &s[s.find("<web-app").unwrap()..];
            let tag = &head[..head.find('>').unwrap()];
            assert!(tag.contains("metadata-complete=\"true\""), "web-app 시작 태그에 있다: {tag}");
            assert_eq!(s.matches("<absolute-ordering/>").count(), 1);
            // 더한 것을 빼면 원본과 한 바이트도 다르지 않다 (줄바꿈도 원본을 따른다)
            let nl = if src.windows(2).any(|w| w == b"\r\n") { "\r\n" } else { "\n" };
            let added = format!(
                "{nl}\t<!-- tomcatrun: skip class scanning at startup (copy of the original web.xml) -->{nl}\t<absolute-ordering/>"
            );
            let had = find_bytes(src, b"metadata-complete").is_some();
            let back = if had { s.clone() } else { s.replacen(" metadata-complete=\"true\"", "", 1) }.replacen(&added, "", 1);
            assert_eq!(back.as_bytes(), src);
            // 두 번 돌려도 더 붙지 않는다
            assert_eq!(dev_web_xml(&out, true).unwrap(), out);
        }
    }

    #[test]
    fn 기동_단계는_앞_표지부터_잰다() {
        // 표지가 순서 없이 들어오고 중간(웹앱 준비 · Spring 루트)이 빠져도 합계는 같다
        let ph = phases_from(1_000, &[(4, 9_000), (0, 3_000), (1, 5_000)]);
        let got: Vec<(&str, u64)> = ph.iter().map(|p| (p.label, p.ms)).collect();
        assert_eq!(got, [("JVM · 에이전트", 2_000), ("톰캣 초기화", 2_000), ("Spring MVC · 마무리", 4_000)]);
        assert_eq!(ph.iter().map(|p| p.ms).sum::<u64>(), 8_000);
    }

    #[test]
    fn 로그_줄은_utf8_이_아니면_ms949_로_읽는다() {
        let (ms949, _, _) = encoding_rs::EUC_KR.encode("컴파일 완료");
        assert_eq!(decode(&ms949), "컴파일 완료");
        assert_eq!(decode("교체됨\r".as_bytes()), "교체됨");
    }
}
