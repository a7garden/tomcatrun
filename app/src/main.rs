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
//                               → 빈 클래스면 Spring 이 빈을 다시 등록하고 URL 매핑을 새로 만든다 (컨트롤러는 7~10초 · JavaWait)
//                               상수 값 · 시그니처가 바뀌면 쓰는 소스도 컴파일한다 (classapi.rs) · 지운 소스의 클래스는 지운다
//   Spring 설정 · web.xml 등    기동 때만 읽는다 → 「재시작해야 반영」 목록에 올린다
//
// 원본은 건드리지 않는다 — 프로젝트 소스 · IDE 설정에는 읽기만 한다.
// SVN 작업복사본 · git 저장소도 읽기만 하고, 「설정 › 버전 관리 › Allow write」를 켠 서버에서만
// 사용자가 누른 Commit · Update · Revert · Resolve (Git 은 Fetch · Push 도)를 svn.exe · git 으로 실행한다 (2026.10.29~ · svncli.rs · gitwrite.rs).
// Commit message 자동 생성은 사용자가 정한 언어 모델 CLI 에 고른 파일의 Diff 를 넘긴다 — 명령 칸이 비면 하지 않는다 (commitmsg.rs).
// 개발에서만 바꾸는 앱 설정은 overlay/ 의 별도 파일로 덧붙이고(tomcatrun.DevTweaks), 쓰는 것은 모두 이 폴더 안이다.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod classapi;
mod commitmsg;
mod eclipse;
mod filtering;
mod git;
mod gitwrite;
mod jarcheck;
mod scancheck;
mod svn;
mod svncli;
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
use std::sync::{Mutex, RwLock};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use notify::{EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager, WindowEvent};

const CREATE_NO_WINDOW: u32 = 0x0800_0000;
const STAMP_FILE: &str = ".tomcatrun-stamp";
const COMPILE_OPTS_FILE: &str = ".tomcatrun-javac"; // 지난 컴파일의 인코딩 · -parameters (compile_opts)

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
    // 2026-10-29~ : 가져올 때 이클립스 서버 설정(server.xml · .launch)에서 읽은 「실행기와 다른 점」. 작업 공간이 있어야 읽으므로 남겨 둔다.
    // 이클립스에서 그 설정을 고쳤다면 다시 가져와야 바뀐다. 프로젝트 .settings 쪽은 남기지 않고 점검할 때마다 다시 읽는다 (server_checks).
    eclipse_warnings: Vec<String>,
    // 2026.10.29~ : 핫리로드를 끄고 띄울 때 쓰는 java. 비면 「설정 › 경로」의 java(전역 java_plain).
    // 「시작하기」가 이클립스 서버가 쓰던 JRE 로 채운다 — 함께 까는 8u181 은 TLS 1.3 · 새 루트 인증서가 없어 외부 연계에서 SSL 오류가 났다.
    // 핫리로드 모드는 DCEVM 에 묶여 있어 늘 java_hot 이다. 실행기 밖의 절대 경로라 path_fields 에 넣지 않는다 (넣으면 빈 값이 기본 경로로 채워진다)
    java: String,
    eclipse_java: String, // 가져올 때 이클립스가 쓰던 java.exe — 「이클립스와 다름」 점검에만 쓴다
    // 2026.10.29~ : server.xml 에 박혀 있던 두 값을 서버 설정으로 옮겼다 — 이클립스에서는 되던 요청이 이 실행기에서만 잘리거나 404 가 났다.
    // 접속 경로는 톰캣 표기 그대로 — 루트는 "", 그 밖은 "/이름" (clean_context_path)
    context_path: String,
    // 톰캣 HTTP Connector 의 maxParameterCount · -1 은 무제한. 옛 tomcatrun.json 에 없으면 0 이 아니라 10000 으로 읽는다 (0 이면 파라미터를 하나도 받지 않는다)
    #[serde(default = "default_max_params")]
    max_parameter_count: i32,
    // 2026.10.29~ : 자바 컴파일 옵션을 이클립스 프로젝트 설정에 맞춘다 — 늘 UTF-8 · -parameters 없이 컴파일해서
    // MS949 소스의 한글 상수가 깨지고, @Param 없는 MyBatis 매퍼가 이 실행기에서만 「Parameter 'x' not found」로 실패했다.
    // 바꾸면 다음 컴파일이 전체를 다시 만든다 (COMPILE_OPTS_FILE). 톰캣이 켜져 있으면 바꾸지 못한다 (set_server).
    encoding: String, // 자바 소스 인코딩 — 빈 값은 UTF-8
    parameters: bool, // 매개변수 이름을 남긴다 (-parameters)
    // 2026.10.29~ : 「겹친 판 치우기」를 눌렀다 — 의존 jar 를 다시 받거나 채우면 폴더가 통째로 갈려 치운 판이 되살아나므로
    // 채운 직후 같은 규칙으로 다시 치운다 (redo_dedup). 「되돌리기」가 끈다.
    deps_dedup: bool,
}

// 톰캣 8.5 기본 · 이클립스 server.xml 에 값이 없을 때와 같다. 2026.10.29 : 1000(최근 톰캣의 기본 server.xml 값) → 10000.
// 로컬 개발 톰캣이라 그리드 일괄 저장이 이클립스와 똑같이 되는 쪽을 골랐다.
const DEFAULT_MAX_PARAMS: i32 = 10000;

fn default_max_params() -> i32 {
    DEFAULT_MAX_PARAMS
}

/// 「설정 › 서버 › 접속 경로」 입력을 톰캣 표기로 — 「/」 · 빈 값은 "", 앞 / 를 붙이고 끝 / 는 뗀다.
/// 공백 · ? · # · \ 가 들어 있으면 주소가 되지 않으므로 받지 않는다.
fn clean_context_path(raw: &str) -> Result<String, String> {
    let t = raw.trim().trim_matches('/');
    if t.is_empty() {
        return Ok(String::new());
    }
    if t.chars().any(|c| c.is_whitespace() || ['?', '#', '\\', '%', ';'].contains(&c)) || t.split('/').any(|s| s.is_empty() || s == "." || s == "..") {
        return Err(format!("접속 경로는 /shop 처럼 적으세요 (공백 · ? · # · \\ 는 쓸 수 없습니다): {raw}"));
    }
    Ok(format!("/{t}"))
}

/// 서버(프로젝트)의 버전 관리 — 「설정 › 버전 관리」에서 고친다.
/// write 를 끄면(기본) 읽기만 하고 Commit · Update 는 명령을 복사해 준다. 켜면 사용자가 누를 때 실행기가 실행한다.
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
struct VcsCfg {
    kind: String,         // auto(폴더를 보고 정한다) · svn · git · both · none
    root: String,         // 작업복사본 · 저장소 폴더. 비면 project
    commit_hint: String,  // Commit message 칸에 흐리게 보일 예시 (팀의 메시지 규칙)
    svn_update: String,   // 「Update 명령 복사」가 만드는 명령 — 앞에 cd 줄이 붙는다
    git_update: String,   // 「Pull 명령 복사」 · Pull 단추가 쓰는 명령
    write: bool,          // Allow write — 이 서버에서 Commit · Update · Revert · Resolve 를 실행한다 (2026.10.29~ · 기본 꺼짐)
    svn_exe: String,      // 비면 PATH → TortoiseSVN · SlikSVN 설치 폴더에서 찾는다
    // ↓ Commit message 규칙 (2026.10.29~ · commitmsg.rs) — 빈 칸은 보지 않는다. 어겨도 막지 않고 확인 창에서 한 번 더 묻는다
    msg_rule: String,        // 자동 생성 때 모델에게 줄 팀 규칙 (자유 글)
    issue_pattern: String,   // 브랜치 · 작업 폴더 이름에서 이슈 번호를 찾는 정규식
    subject_pattern: String, // 제목이 맞아야 하는 정규식
    subject_max: u32,        // 제목 최대 글자 수 (0 이면 보지 않는다)
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
            // 작업복사본이 운영 저장소와 바로 이어져 있을 수 있어 쓰기는 사용자가 서버마다 켠다
            write: false,
            svn_exe: String::new(),
            msg_rule: String::new(),
            // 「FDR-112」 · 「ABC-7」 꼴 — Jira 같은 이슈 키
            issue_pattern: r"[A-Z][A-Z0-9]+-\d+".into(),
            subject_pattern: String::new(),
            subject_max: 0,
        }
    }
}

impl VcsCfg {
    fn rule(&self) -> commitmsg::Rule {
        commitmsg::Rule {
            text: self.msg_rule.clone(),
            example: self.commit_hint.clone(),
            issue_pattern: self.issue_pattern.clone(),
            subject_pattern: self.subject_pattern.clone(),
            subject_max: self.subject_max,
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
    msg_cmd: String,      // Commit message 생성 명령 (예: claude -p) — 비우면 자동 생성을 숨긴다. 고른 파일의 Diff 가 이 명령으로 나간다
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
    profile_start: bool,  // 기동 표본 — 켜는 동안 2초마다 스레드를 떠서 어디서 시간을 쓰는지 남긴다 (start_sampler)
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

// 자바 9 이상으로 띄울 때 — 톰캣 8.5 의 catalina.bat 와 같은 값 (JDK_JAVA_OPTIONS)
const JDK9_ARGS: [&str; 3] = [
    "--add-opens=java.base/java.lang=ALL-UNNAMED",
    "--add-opens=java.base/java.io=ALL-UNNAMED",
    "--add-opens=java.rmi/sun.rmi.transport=ALL-UNNAMED",
];

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
            // 2026.10.29 : 비워 둔다 — 소스가 밖으로 나가는 일이라 사용자가 명령을 적어야 켜진다
            msg_cmd: String::new(),
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
            // jstack 이 붙을 때마다 JVM 이 잠깐 멈춰 기동이 조금 늘어난다 — 잴 때만 켠다
            profile_start: false,
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
        eclipse_warnings: vec![],
        java: String::new(),
        eclipse_java: String::new(),
        context_path: String::new(),
        max_parameter_count: DEFAULT_MAX_PARAMS,
        encoding: String::new(),
        parameters: false,
        deps_dedup: false,
        // 2026-10-28 : 「http://localhost:8080/」 대신 「/」. 「/」로 시작하는 주소는 화면이 열 때 그 서버의 HTTP 포트를 붙이므로
        // 포트를 바꿔도 바로가기가 따라간다 (서버를 더할 때 포트를 올려도 링크를 다시 만들 필요가 없다)
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
            (c, Some("처음 실행입니다. 「설정 › 서버」에서 서버 이름 · 프로젝트 폴더 · 포트 · 바로가기를 정하세요. 이 PC 에 맞는 값은 실행기가 알아내지 않습니다.".into()))
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
    java_wait: Option<JavaWait>,    // 컴파일한 자바가 실행 중인 앱에 다 들어가기를 기다리는 중 (watch_java)
    session_had: Option<u64>,       // 이번 기동 전에 남아 있던 세션 파일 크기 (없었거나 로그인 유지를 껐으면 None)
    session_err: Option<String>,    // 톰캣이 세션을 저장 · 되살리다 낸 오류의 첫 예외 줄 (빈 값이면 그 줄을 기다리는 중)
    mem: Mem,
    compiling: bool,
    last_compile: Option<CompileResult>,
    restart_needed: Vec<Pending>,
}

/// 자바 반영의 진행 — 컴파일 → 클래스 교체(HotswapAgent) → Spring 이 빈을 다시 등록하고 URL 매핑을 새로 만든다.
/// 컨트롤러는 마지막 단계가 교체 뒤 7~10초 걸린다 (2026-10-27 로그). 그 끝을 알리지 않으면 먼저 눌러 보고 「안 들어갔다」고 본다.
struct JavaWait {
    id: u64,                 // 컴파일마다 새로 — 지켜보는 스레드가 제 것인지 가린다
    since: Instant,          // 컴파일을 시작한 때
    reloaded: bool,          // 「Reloading classes」 를 받았다
    beans: Vec<String>,      // 다시 등록한 Spring 빈
    last: Instant,           // 마지막으로 관련 줄을 받은 때
}

/// 서버 하나의 실행 상태
struct Rt {
    last_reload: Mutex<String>, // 마지막 「Reloading classes [...]」 의 클래스 (교체 실패 알림에 이름을 붙인다)
    inner: Mutex<Inner>,
    compile_lock: Mutex<()>,
    compile_tx: Sender<bool>, // true = 전체 컴파일
    // 2026.10.29 : 줄마다 파일에 쓰던 것을 버퍼로 — 기동 때 수천 줄이 줄마다 시스템 호출이 되었다. monitor 가 3초마다 비운다
    console: Mutex<Option<std::io::BufWriter<File>>>,
    deps_busy: AtomicBool,
    live_mappers: Mutex<Vec<String>>,
    res_gen: AtomicU64, // src/main 의 java · webapp 밖을 저장할 때마다 올린다 — 리소스 필터링 점검을 다시 세는 표지
    filter_check: Mutex<Option<(String, Option<Check>)>>, // 리소스 필터링 점검 (pom · m2e 설정 시각 · res_gen 이 같으면 다시 세지 않는다)
    scan_jars: Mutex<Option<(String, scancheck::JarReport)>>, // 기동 탐색 점검의 jar 쪽 (jar 이름 · 크기 · 시각이 같으면 다시 열지 않는다)
    scan_src: Mutex<Option<(u64, Vec<String>)>>,              // 같은 점검의 소스 쪽 (컴파일 표지가 같으면 다시 읽지 않는다)
    svn_stamp: AtomicU64, // 작업복사본 .svn/wc.db 의 수정 시각 — 바뀌면(커밋 · 업데이트) 버전 관리 화면을 다시 읽게 한다
    git_stamp: AtomicU64, // .git 의 index · HEAD · FETCH_HEAD 중 가장 늦은 수정 시각 — 같은 용도
    vcs_busy: AtomicBool, // Commit · Update 같은 쓰기가 도는 중 — 한 서버에 하나씩, 그동안 위 두 표지로 다시 읽게 하지 않는다
}

struct Shared {
    cfg: Mutex<Config>,
    // 서버를 더하면 실행 중에 늘어난다 (줄지는 않는다). 하나씩 앱 끝까지 살려 두어 참조를 잠금 밖으로 넘긴다.
    // 늘릴 때는 cfg.servers 보다 먼저 늘린다 — 화면 상태가 cfg.servers 의 번호로 rts 를 찾는다.
    rts: RwLock<Vec<&'static Rt>>,
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

fn rt(app: &AppHandle, i: usize) -> &'static Rt {
    shared(app).rts.read().unwrap()[i]
}

fn rt_count(app: &AppHandle) -> usize {
    shared(app).rts.read().unwrap().len()
}

fn new_rt() -> (&'static Rt, Receiver<bool>) {
    let (tx, rx) = mpsc::channel();
    let r = Rt {
        last_reload: Mutex::new(String::new()),
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
            java_wait: None,
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
        res_gen: AtomicU64::new(0),
        filter_check: Mutex::new(None),
        scan_jars: Mutex::new(None),
        scan_src: Mutex::new(None),
        svn_stamp: AtomicU64::new(0),
        git_stamp: AtomicU64::new(0),
        vcs_busy: AtomicBool::new(false),
    };
    (Box::leak(Box::new(r)), rx)
}

// 서버가 n 개가 되도록 실행 상태와 컴파일 줄을 늘린다 — 서버를 더해도 실행기를 다시 열지 않는다
fn grow_rts(app: &AppHandle, n: usize) {
    let mut rts = shared(app).rts.write().unwrap();
    while rts.len() < n {
        let (r, rx) = new_rt();
        let i = rts.len();
        rts.push(r);
        let a = app.clone();
        thread::spawn(move || compile_worker(a, i, rx));
    }
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
    warn: bool, // 준비는 됐지만 사용자가 알아 둘 것 (ok 는 참) — 「준비 안 된 항목」에 세지 않고 시작도 막지 않는다
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
    context_path: String,        // 「/」로 시작하는 바로가기에 붙인다
    max_parameter_count: i32,
    encoding: String,
    parameters: bool,
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
    svn_cli: String,             // 찾은 svn.exe 와 판 (또는 못 찾은 까닭) — 설정 › 버전 관리에 보인다
    svn_block: Option<String>,   // Allow write 를 켰어도 SVN 쓰기를 못 하는 까닭 — 있으면 쓰기 단추를 잠근다
    java: String,                // 서버에 정한 java (핫리로드 끔) — 비면 전역
    java_label: String,          // 핫리로드를 끄고 띄우면 쓰는 java 의 판 (설정 › 서버 › java 칸 아래에 보인다)
    eclipse_java: String,        // 이클립스가 쓰던 java (가져올 때 찾았으면)
    eclipse_java_label: String,
    java_diff: Option<String>,   // 이클립스와 다른 자바로 뜨면 그 설명 — 알림 줄에 띄운다
    dedup: DedupSnap,
}

/// 같은 jar 다른 판 — 화면이 「겹친 판 치우기」 · 「되돌리기」 단추와 펼침 목록을 그린다
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct DedupSnap {
    movable: usize,     // 단추로 치울 수 있는 의존 jar 수
    manual: usize,      // 프로젝트 WEB-INF/lib 안에서 겹쳐 사람이 지워야 하는 라이브러리 수
    lines: Vec<String>, // 라이브러리마다 「남김 · 치움」 한 줄
    aside: Vec<String>, // 치워 둔 jar (deps.dup)
    on: bool,           // 다시 받아도 다시 치운다
}

/// 「설정 › 경로」 — 모든 서버가 함께 쓰는 것
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Paths {
    tomcat_home: String,
    javac: String,
    java_plain: String,
    editor: String,
    #[serde(default)]
    msg_cmd: String,
    tld_scan: String,
}

/// 「설정 › 서버」 한 칸 — 서버 하나의 이름 · 프로젝트 · 포트 · 바로가기 · JVM 인자
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
    #[serde(default)]
    java: String,
    #[serde(default)]
    context_path: String,
    #[serde(default = "default_max_params")]
    max_parameter_count: i32,
    #[serde(default)]
    encoding: String,
    #[serde(default)]
    parameters: bool,
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
    profile_start: bool,
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
        Check { key: "tomcat", label: "톰캣 8.5", ok: ex(&format!("{}/bin/bootstrap.jar", c.tomcat_home)), detail: c.tomcat_home.clone(), warn: false },
        Check { key: "jdk", label: "JDK 8 (javac)", ok: ex(&c.javac) && ex(&c.java_plain), detail: c.javac.clone(), warn: false },
        Check { key: "dcevm", label: "DCEVM JDK 8", ok: ex(&c.java_hot) && dcevm, detail: c.java_hot.clone(), warn: false },
        Check { key: "agent", label: "HotswapAgent", ok: ex(&c.hotswap_agent), detail: c.hotswap_agent.clone(), warn: false },
        Check {
            key: "helper",
            label: "상주 컴파일러",
            ok: helper_dir().join("TomcatrunCompiler.class").exists(),
            detail: p(&helper_dir()),
            warn: false,
        },
        Check {
            key: "ecj",
            label: "ECJ (이클립스 컴파일러)",
            ok: ecj_jar(c).is_some(),
            detail: ecj_jar(c).map(|j| p(&j)).unwrap_or_else(|| "톰캣 lib 에 없음 · javac 로 컴파일".into()),
            warn: false,
        },
        Check {
            key: "overlay",
            label: "개발 전용 앱 설정",
            ok: overlay_dir().join("tomcatrun/DevTweaks.class").exists()
                && overlay_dir().join("spring/context-zz-tomcatrun.xml").exists()
                && overlay_dir().join("egovframework/springmvc/application-servlet.xml").exists(),
            detail: p(&overlay_dir()),
            warn: false,
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

/// 이 서버를 띄울 java — 핫리로드면 DCEVM, 아니면 plain_java
fn launch_java(c: &Config, s: &ServerCfg) -> String {
    if c.hot_reload { c.java_hot.clone() } else { plain_java(c, s) }
}

/// 핫리로드를 끄고 띄울 java — 서버의 java, 비었으면 「설정 › 경로」의 java
fn plain_java(c: &Config, s: &ServerCfg) -> String {
    if s.java.trim().is_empty() { c.java_plain.clone() } else { s.java.trim().replace('\\', "/") }
}

/// 핫리로드 모드의 인증서 목록 — DCEVM 의 cacerts 에 Windows 루트 인증서 저장소를 더해 <base>/temp/cacerts 로 만든다.
/// 2026.10.29 : DCEVM 8u181 의 cacerts 는 2018년 판이라 GlobalSign Root R3 가 없어 브이월드(api.vworld.kr) 호출이 PKIX 오류로 막혔다.
/// Windows 저장소만 쓰지 않는 것은 Windows 가 루트를 쓸 때가 되어서야 내려받아 채워서 cacerts 에 있던 것이 빠질 수 있어서다.
/// Windows 저장소가 기동마다 바뀔 수 있으니 기동마다 다시 만든다 (1초 안쪽). 돌려주는 값은 (경로 · 더한 수) 이고, 못 만들면 Err 의 까닭과 함께 기본 cacerts 로 뜬다.
fn merged_trust_store(java_hot: &str, base: &str) -> Result<(String, usize), String> {
    let bin = Path::new(java_hot).parent().ok_or("java 경로가 비었습니다")?;
    let src = bin.parent().ok_or("JDK 폴더를 찾지 못했습니다")?.join("jre/lib/security/cacerts");
    let dst = Path::new(base).join("temp/cacerts");
    // 다 만든 뒤에만 바꿔 넣는다 — 도중에 실패한 반쪽 목록으로 뜨면 기본보다 못하다
    let tmp = Path::new(base).join("temp/cacerts.tmp");
    fs::copy(&src, &tmp).map_err(|e| format!("{} 을 복사하지 못했습니다: {e}", p(&src)))?;
    let out = Command::new(bin.join("keytool.exe"))
        .args(["-J-Duser.language=en", "-importkeystore", "-srcstoretype", "Windows-ROOT", "-srckeystore", "NONE", "-destkeystore"])
        .arg(&tmp)
        .args(["-deststorepass", "changeit", "-noprompt"])
        .stdin(Stdio::null())
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .map_err(|e| format!("keytool 을 실행하지 못했습니다: {e}"))?;
    let text = String::from_utf8_lossy(&out.stdout).into_owned() + &String::from_utf8_lossy(&out.stderr);
    if !out.status.success() {
        let _ = fs::remove_file(&tmp);
        return Err(format!("keytool 이 실패했습니다: {}", text.lines().find(|l| !l.trim().is_empty()).unwrap_or("").trim()));
    }
    fs::rename(&tmp, &dst).map_err(|e| format!("{} 을 쓰지 못했습니다: {e}", p(&dst)))?;
    Ok((p(&dst), text.matches("successfully imported").count()))
}

/// java.exe 의 판 (주 번호 · 보일 글자) — JDK 의 release 파일에서 읽는다. JDK 8 은 jre/bin/java.exe 일 수도 있어 한 단계 더 올라가 본다.
/// 상태를 그릴 때마다 부르므로 java 를 띄우지 않고 파일만 읽는다 (경로 · 고친 시각으로 담아 둔다).
fn java_version(java: &str) -> Option<(u32, String)> {
    static CACHE: Mutex<Vec<(String, u64, Option<(u32, String)>)>> = Mutex::new(Vec::new());
    let bin = Path::new(java).parent()?;
    let release = [bin.parent()?.join("release"), bin.parent()?.parent()?.join("release")].into_iter().find(|f| f.is_file())?;
    let stamp = mtime_ms(&release).unwrap_or(0);
    let key = p(&release);
    let mut cache = CACHE.lock().unwrap();
    if let Some(hit) = cache.iter().find(|(k, t, _)| *k == key && *t == stamp) {
        return hit.2.clone();
    }
    let text = fs::read_to_string(&release).unwrap_or_default();
    let v = text.lines().find_map(|l| l.strip_prefix("JAVA_VERSION=")).map(|v| v.trim().trim_matches('"').to_string()).and_then(|v| parse_java_version(&v));
    cache.retain(|(k, _, _)| *k != key);
    cache.push((key, stamp, v.clone()));
    v
}

// 「1.8.0_181」 → (8, 「8u181」) · 「17.0.8」 → (17, 「17.0.8」) · 「11」 → (11, 「11」)
fn parse_java_version(v: &str) -> Option<(u32, String)> {
    let num = |s: &str| s.chars().take_while(char::is_ascii_digit).collect::<String>().parse::<u32>().ok();
    if let Some(rest) = v.strip_prefix("1.") {
        let major = num(rest)?;
        let update = v.split('_').nth(1).and_then(num);
        return Some((major, update.map_or(format!("{major}"), |u| format!("{major}u{u}"))));
    }
    Some((num(v)?, v.to_string()))
}

/// 기동 인자를 고를 주 번호. release 가 없으면 폴더 이름(jdk-17 · jre11 · temurin-21…)으로 짐작하고, 그래도 모르면 8 로 본다 (지금까지와 같은 인자).
fn java_major(java: &str) -> u32 {
    if let Some((m, _)) = java_version(java) {
        return m;
    }
    Path::new(java)
        .components()
        .filter_map(|c| {
            let name = c.as_os_str().to_string_lossy().to_ascii_lowercase();
            let at = name.find(|ch: char| ch.is_ascii_digit())?;
            let head = &name[..at];
            if !["jdk", "jre", "java", "temurin", "openjdk", "zulu", "corretto"].iter().any(|x| head.trim_end_matches(['-', '_', ' ', '.']).ends_with(x)) {
                return None;
            }
            parse_java_version(&name[at..]).map(|v| v.0)
        })
        .find(|m| (8..100).contains(m))
        .unwrap_or(8)
}

fn java_label(java: &str) -> String {
    java_version(java).map(|v| v.1).unwrap_or_else(|| "판 모름".into())
}

/// 이클립스와 다른 java 로 뜨는지 — 가져올 때 이클립스 JRE 를 찾은 서버만 본다
fn java_diff(c: &Config, s: &ServerCfg) -> Option<String> {
    let ej = s.eclipse_java.trim();
    if ej.is_empty() {
        return None;
    }
    let ours = launch_java(c, s);
    let same = |a: &str, b: &str| a.replace('\\', "/").eq_ignore_ascii_case(&b.replace('\\', "/"));
    if same(ej, &ours) {
        return None;
    }
    let (theirs, mine) = (java_label(ej), java_label(&ours));
    // 경로만 다르고 판이 같으면 (같은 JDK 를 복사해 둔 것 등) 알리지 않는다
    if java_version(ej).is_some() && theirs == mine {
        return None;
    }
    Some(if c.hot_reload {
        // 판 뒤에 조사를 붙이지 않는다 — 「17.0.8 을」 · 「11.0.2 를」 처럼 끝 숫자마다 달라진다
        format!("이클립스가 쓰던 자바는 {theirs} 입니다 · 핫리로드 모드는 {mine} 입니다 (DCEVM) — 외부 연계에서 SSL 오류(SSLHandshakeException)나 UnsupportedClassVersionError 가 나면 핫리로드를 끄고 띄워 보세요. 끄면 이클립스와 같은 자바로 뜹니다")
    } else {
        format!("이클립스가 쓰던 자바는 {theirs} 입니다 · 이 서버는 {mine} 입니다 ({ours}) — 같게 하려면 설정 › 서버 › java 칸에 이클립스 경로({ej})를 적으세요")
    })
}

fn helper_dir() -> PathBuf {
    home_dir().join("helper")
}

fn overlay_dir() -> PathBuf {
    home_dir().join("overlay")
}

// pom 의 리소스 필터링 — resources 를 훑으므로 pom · m2e 설정 · 저장(res_gen)이 바뀔 때만 다시 센다. 필터링을 쓰지 않으면 줄이 없다.
fn filter_check(s: &ServerCfg, r: &Rt) -> Option<Check> {
    if s.project.trim().is_empty() {
        return None;
    }
    let proj = Path::new(&s.project);
    let key = format!(
        "{}|{}|{}|{}",
        s.project,
        mtime_ms(&proj.join("pom.xml")).unwrap_or(0),
        mtime_ms(&proj.join(filtering::M2E_PREFS)).unwrap_or(0),
        r.res_gen.load(Ordering::Relaxed)
    );
    let mut cache = r.filter_check.lock().unwrap();
    if let Some((k, c)) = cache.as_ref() {
        if *k == key {
            return c.clone();
        }
    }
    let c = filtering::scan(proj).map(|rep| Check { key: "filtering", label: "리소스 필터링", ok: true, detail: rep.detail(), warn: rep.warn() });
    *cache = Some((key, c.clone()));
    c
}

// 기동 탐색 점검의 재료 — jar 쪽은 jar 이름 · 크기 · 시각이, 소스 쪽은 컴파일 표지가 같으면 지난 결과를 쓴다
fn scan_report(s: &ServerCfg, r: &Rt) -> (scancheck::JarReport, Vec<String>) {
    let own = own_lib(&s.project);
    let deps = PathBuf::from(&s.deps_dir);
    let mut key = String::new();
    for dir in [&own, &deps] {
        let Ok(rd) = fs::read_dir(dir) else { continue };
        for e in rd.filter_map(Result::ok) {
            let n = e.file_name().to_string_lossy().to_string();
            if n.to_ascii_lowercase().ends_with(".jar") {
                let m = e.metadata().ok();
                let t = m.as_ref().and_then(|m| m.modified().ok()).and_then(|t| t.duration_since(UNIX_EPOCH).ok()).map_or(0, |d| d.as_millis());
                key.push_str(&format!("{n}|{}|{t};", m.map_or(0, |m| m.len())));
            }
        }
    }
    let jars = {
        let mut cache = r.scan_jars.lock().unwrap();
        match cache.as_ref() {
            Some((k, j)) if *k == key => j.clone(),
            _ => {
                let j = scancheck::scan_jars(&[&own, &deps]);
                *cache = Some((key, j.clone()));
                j
            }
        }
    };
    let stamp = read_stamp(&s.classes_dir);
    let src = {
        let mut cache = r.scan_src.lock().unwrap();
        match cache.as_ref() {
            Some((k, v)) if *k == stamp => v.clone(),
            _ => {
                let v = scancheck::src_hits(&Path::new(&s.project).join("src/main/java"));
                *cache = Some((stamp, v.clone()));
                v
            }
        }
    };
    (jars, src)
}

// 「TLD jar」 칸 · 「클래스 전체 탐색 건너뛰기」를 이 서버의 jar · 소스와 견준다.
// 2026-10-29 실측 : 둘 다 꺼 둔 서버는 웹앱 준비가 17~36초, 켠 서버는 2초 안팎이었다 (같은 앱).
// 줄이 생기는 것은 좁힐 수 있을 때(scanHint · 「탐색 줄이기」 단추)와 지금 값이 이 서버에 맞지 않을 때(scanWarn)뿐이다.
fn scan_check(c: &Config, s: &ServerCfg, r: &Rt) -> Option<Check> {
    if s.project.trim().is_empty() {
        return None;
    }
    let (jars, src) = scan_report(s, r);
    let mut lines: Vec<String> = Vec::new();
    let mut can_narrow = false;
    let cfg_pats: Vec<String> = c.tld_scan.split(',').map(|x| x.trim().to_ascii_lowercase()).filter(|x| !x.is_empty()).collect();
    let mut pats: Vec<String> = jars.tld.iter().map(|j| scancheck::tld_pattern(j)).collect();
    pats.sort();
    pats.dedup();
    if cfg_pats.is_empty() {
        if !pats.is_empty() {
            lines.push(format!("TLD jar 칸이 비어 있어 기동마다 jar {}개를 모두 훑습니다 · 태그 라이브러리가 든 jar 는 {}개입니다: {}", jars.total, pats.len(), pats.join(", ")));
            can_narrow = true;
        }
    } else {
        let missing: Vec<&String> = jars.tld.iter().filter(|j| !cfg_pats.iter().any(|p| glob(p, &j.to_ascii_lowercase()))).collect();
        if !missing.is_empty() {
            let names: Vec<&str> = missing.iter().map(|m| m.as_str()).collect();
            lines.push(format!("TLD jar 칸에 빠진 jar: {} · 그 태그를 쓰는 JSP 가 「태그를 찾을 수 없다」로 깨집니다", names.join(", ")));
            can_narrow = true;
        }
    }
    let blockers = scancheck::skip_blockers(&jars, &src);
    if !c.skip_scan {
        if blockers.is_empty() {
            lines.push("클래스 전체 탐색 건너뛰기를 켜도 됩니다 · 애너테이션으로 등록하는 서블릿 · 필터 · 초기화 클래스가 소스와 jar 에 없습니다".into());
            can_narrow = true;
        }
    } else if !blockers.is_empty() {
        lines.push("클래스 전체 탐색 건너뛰기가 켜져 있지만, 이 서버에서는 건너뛰면 빠지는 것이 있습니다 · 기동이 되어도 그 서블릿 · 필터가 없을 수 있으니 끄세요".into());
        lines.extend(blockers);
    }
    if lines.is_empty() {
        return None;
    }
    if can_narrow {
        lines.push("「탐색 줄이기」를 누르면 다음 기동부터 들어갑니다 · 두 값은 모든 서버가 함께 씁니다".into());
    }
    Some(Check { key: if can_narrow { "scanHint" } else { "scanWarn" }, label: "기동 탐색", ok: true, detail: lines.join("\n"), warn: true })
}

// 「탐색 줄이기」 — 모든 서버의 TLD jar 를 칸에 모으고, 모든 서버가 건너뛰어도 되면 클래스 전체 탐색 건너뛰기를 켠다.
// 두 값이 서버 공통이라 한 서버만 보고 좁히면 다른 서버의 JSP 가 깨진다.
#[tauri::command]
fn apply_scan_hint(app: AppHandle) -> Result<String, String> {
    let servers = shared(&app).cfg.lock().unwrap().servers.clone();
    let mut tld: Vec<String> = Vec::new(); // TLD 가 든 jar 이름 (모든 서버)
    let mut all_safe = true;
    for (i, s) in servers.iter().enumerate() {
        if s.project.trim().is_empty() {
            continue;
        }
        let (jars, src) = scan_report(s, rt(&app, i));
        tld.extend(jars.tld.iter().map(|j| j.to_ascii_lowercase()));
        all_safe &= scancheck::skip_blockers(&jars, &src).is_empty();
    }
    let mut done: Vec<String> = Vec::new();
    {
        let mut c = shared(&app).cfg.lock().unwrap();
        let mut cur: Vec<String> = c.tld_scan.split(',').map(|x| x.trim().to_string()).filter(|x| !x.is_empty()).collect();
        // 비어 있으면 모든 jar 를 보는 것이라 빠진 것이 없다 — 그때는 찾은 것으로 좁히고, 적혀 있으면 빠진 것만 더한다
        let was_all = cur.is_empty();
        let mut added: Vec<String> = Vec::new();
        for j in tld.iter().filter(|j| was_all || !cur.iter().any(|q| glob(&q.to_ascii_lowercase(), j))) {
            let pat = scancheck::tld_pattern(j);
            if !added.contains(&pat) {
                added.push(pat);
            }
        }
        if !added.is_empty() {
            cur.extend(added.iter().cloned());
            c.tld_scan = cur.join(",");
            done.push(if was_all { format!("TLD jar 를 {}개로 좁혔습니다", cur.len()) } else { format!("TLD jar 칸에 {}개를 더했습니다", added.len()) });
        }
        if all_safe && !c.skip_scan {
            c.skip_scan = true;
            done.push("클래스 전체 탐색 건너뛰기를 켰습니다".into());
        }
        if done.is_empty() {
            return Ok("바꿀 것이 없습니다.".into());
        }
        save_config(&c).map_err(|e| format!("설정을 저장하지 못했습니다: {e}"))?;
    }
    emit_state(&app);
    let msg = format!("{} · 다음 기동부터 들어갑니다.", done.join(" · "));
    log(&app, None, "app", "", msg.clone());
    Ok(msg)
}

// 프로젝트 WEB-INF/lib — 프로젝트를 정하지 않았으면 빈 경로 (실행기 폴더 기준 상대 경로를 읽지 않게 한다)
fn own_lib(project: &str) -> PathBuf {
    if project.trim().is_empty() { PathBuf::new() } else { Path::new(project).join("src/main/webapp/WEB-INF/lib") }
}

fn dedup_snap(s: &ServerCfg, dups: &[jarcheck::Dup]) -> DedupSnap {
    DedupSnap {
        movable: dups.iter().map(|d| d.drop.len()).sum(),
        manual: dups.iter().filter(|d| !d.manual.is_empty()).count(),
        lines: dups.iter().map(jarcheck::plan_line).collect(),
        aside: jarcheck::aside_jars(Path::new(&s.deps_dir)),
        on: s.deps_dedup,
    }
}

fn server_checks(c: &Config, s: &ServerCfg, r: &Rt, dups: &[jarcheck::Dup], aside: &[String]) -> Vec<Check> {
    let jars = fs::read_dir(&s.deps_dir)
        .map(|r| r.filter_map(Result::ok).filter(|e| e.path().extension().map_or(false, |x| x == "jar")).count())
        .unwrap_or(0);
    // pom 을 고친 뒤 jar 를 다시 받지 않았으면 알린다
    let pom = mtime_ms(&Path::new(&s.project).join("pom.xml")).unwrap_or(0);
    let deps_fresh = read_stamp(&s.deps_dir) >= pom;
    let compiled = Path::new(&s.classes_dir).join(STAMP_FILE).exists();
    let mut out = vec![
        Check { key: "project", label: "소스", ok: Path::new(&s.project).join("src/main/webapp").exists(), detail: s.project.clone(), warn: false },
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
            warn: false,
        },
        Check {
            key: "classes",
            label: "컴파일 결과",
            ok: compiled,
            detail: if compiled { s.classes_dir.clone() } else { "아직 없음 · 처음 시작할 때 전체 컴파일".into() },
            warn: false,
        },
    ];
    // 같은 라이브러리의 다른 판 — 시작은 되므로 노란 줄. 키를 deps 와 나눈 것은 화면이 deps 줄 하나로 「다시 받기」 강조 · 「자동으로 채우기」 알림을 걸기 때문이다
    if !dups.is_empty() {
        out.push(Check { key: "depsDup", label: "같은 jar 다른 판", ok: true, detail: jarcheck::detail(dups), warn: true });
    }
    // 치워 둔 판 — 겹침이 사라져도 남겨 보여야 되돌릴 곳이 있다
    if !aside.is_empty() {
        let mut d = vec![format!("{}개 · 의존 jar 폴더 옆 deps.dup 에 있습니다{}", aside.len(), if s.deps_dedup { " · 다시 받아도 다시 치웁니다" } else { "" })];
        d.extend(aside.iter().cloned());
        out.push(Check { key: "depsAside", label: "치운 jar", ok: true, detail: d.join("\n"), warn: false });
    }
    // SVN 쓰기를 켠 서버만 — 끈 서버는 svn.exe 가 없어도 된다
    if s.vcs.write && vcs_kinds(s).contains(&"svn") {
        let block = svn_block(s);
        let detail = block.clone().unwrap_or_else(|| svncli::probe(&s.vcs.svn_exe).map(|p| format!("svn {} · {}", p.version, p.exe)).unwrap_or_default());
        out.push(Check { key: "svn", label: "svn.exe (SVN 쓰기)", ok: block.is_none(), detail, warn: false });
    }
    out.extend(filter_check(s, r));
    out.extend(scan_check(c, s, r));
    // 이클립스와 다르게 도는 것 — 가져올 때 남긴 서버 설정 쪽 + 지금 프로젝트 폴더의 .settings (pom 리소스 필터링은 위 줄이 따로 센다)
    let mut diff = s.eclipse_warnings.clone();
    diff.extend(java_diff(c, s));
    if !s.project.trim().is_empty() {
        diff.extend(eclipse::project_settings(Path::new(&s.project), None).warnings(&s.encoding, s.parameters));
    }
    if !diff.is_empty() {
        if !s.eclipse_warnings.is_empty() || !s.eclipse_java.is_empty() {
            diff.push("이클립스 서버 설정을 고쳤다면 다시 가져와야 이 목록이 바뀝니다 (설정 › 서버 › 이클립스에서 가져오기)".into());
        }
        out.push(Check { key: "eclipse", label: "이클립스와 다름", ok: true, detail: diff.join("\n"), warn: true });
    }
    out
}

// Allow write 를 켠 SVN 서버에서 쓰기를 못 하는 까닭 — svn.exe 가 없거나, 판 · 작업복사본 형식이 맞지 않으면
fn svn_block(s: &ServerCfg) -> Option<String> {
    if !s.vcs.write || !vcs_kinds(s).contains(&"svn") {
        return None;
    }
    match svncli::probe(&s.vcs.svn_exe) {
        Err(e) => Some(e),
        Ok(p) => svncli::block_reason(&p, svn::wc_format(Path::new(&vcs_root(s)))),
    }
}

fn snapshot(app: &AppHandle) -> Snapshot {
    let c = app.state::<Shared>().cfg.lock().unwrap().clone();
    let servers = c
        .servers
        .iter()
        .enumerate()
        .map(|(n, s)| {
            let r = rt(app, n);
            // 2026.10.29 : 폴더 · 파일을 읽는 점검을 상태 잠금 밖에서 먼저 한다 — 쥔 채로 읽으면 그동안 로그 읽기 · 컴파일이 기다린다
            let dups = jarcheck::duplicates(&own_lib(&s.project), Path::new(&s.deps_dir));
            let dedup = dedup_snap(s, &dups);
            let checks = server_checks(&c, s, r, &dups, &dedup.aside);
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
                context_path: s.context_path.clone(),
                max_parameter_count: s.max_parameter_count,
                encoding: s.encoding.clone(),
                parameters: s.parameters,
                status: i.status,
                pid: i.child.as_ref().map(|c| c.id()),
                started_at_ms: i.started_at_ms,
                startup_ms: i.startup_ms,
                mode: i.mode.clone(),
                compiling: i.compiling,
                last_compile: i.last_compile.clone(),
                restart_needed: i.restart_needed.clone(),
                deps_busy: r.deps_busy.load(Ordering::Relaxed),
                checks,
                java: s.java.clone(),
                java_label: java_label(&plain_java(&c, s)),
                eclipse_java: s.eclipse_java.clone(),
                eclipse_java_label: if s.eclipse_java.is_empty() { String::new() } else { java_label(&s.eclipse_java) },
                java_diff: java_diff(&c, s),
                phases: i.phases.clone(),
                waiting: i.waiting,
                mem: i.mem,
                vcs: s.vcs.clone(),
                vcs_kinds: vcs_kinds(s),
                vcs_root: vcs_root(s),
                svn_cli: match svncli::probe(&s.vcs.svn_exe) {
                    Ok(p) => format!("svn {} · {}", p.version, p.exe),
                    Err(e) => e,
                },
                svn_block: svn_block(s),
                dedup,
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
        profile_start: c.profile_start,
        ignore: c.ignore.clone(),
        paths: Paths {
            tomcat_home: c.tomcat_home.clone(),
            javac: c.javac.clone(),
            java_plain: c.java_plain.clone(),
            editor: c.editor.clone(),
            msg_cmd: c.msg_cmd.clone(),
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

// HotswapAgent 가 클래스를 실행 중인 JVM 에 넣지 못했다는 줄.
// 2026-10-28 : 「HOTSWAP AGENT」 와 함께 있는 줄만 찾았는데, 실패는 머리 없는 예외 줄로만 나와 한 번도 잡지 못했다.
// 그래서 교체가 실패해도(익명 · 안쪽 클래스 구조가 바뀐 경우 등) 재시작 알림이 뜨지 않고 옛 코드가 그대로 돌았다.
fn is_redefine_failure(t: &str) -> bool {
    t.contains("Unable to redefine classes") || t.contains("class redefinition failed")
}

// 「Reloading classes [a.b.C$1, a.b.C, a.b.D]」 → 「C 외 1개」 (안쪽 클래스는 바깥 클래스로 묶는다)
fn reload_names(t: &str) -> Option<String> {
    let at = t.find("Reloading classes [")? + "Reloading classes [".len();
    let body = &t[at..t[at..].find(']').map_or(t.len(), |e| at + e)];
    let mut names: Vec<String> = Vec::new();
    for c in body.split(',').map(str::trim).filter(|c| !c.is_empty()) {
        let simple = c.rsplit('.').next().unwrap_or(c).split('$').next().unwrap_or(c).to_string();
        if !names.contains(&simple) {
            names.push(simple);
        }
    }
    match names.len() {
        0 => None,
        1 => Some(names[0].clone()),
        n => Some(format!("{} 외 {}개", names[0], n - 1)),
    }
}

// 「Registering Spring bean 'fdrController'」 → fdrController
fn bean_name(t: &str) -> Option<&str> {
    let at = t.find("Registering Spring bean '")? + "Registering Spring bean '".len();
    let end = t[at..].find('\'')?;
    Some(&t[at..at + end])
}

// 「a 외 n개」 — 반영 기록 한 줄에 이름을 다 늘어놓지 않는다
fn some_names(v: &[String]) -> String {
    match v.len() {
        0 => String::new(),
        1 => v[0].clone(),
        n => format!("{} 외 {}개", v[0], n - 1),
    }
}

/// 톰캣 로그 한 줄로 자바 반영 단계를 넘긴다 (JavaWait). 교체를 받으면 그 자리에서 한 줄 남기고, 끝은 watch_java 가 알린다.
fn note_java_line(app: &AppHandle, i: usize, text: &str) {
    let is_agent = text.starts_with("HOTSWAP AGENT");
    // URL 매핑을 다시 만드는 동안 Spring 이 「Mapped "{[/a.do]}" …」 를 수백 줄 남긴다 — 그동안은 아직 끝나지 않은 것이다
    if !is_agent && !text.contains("Mapped \"{") {
        return;
    }
    let mut inner = rt(app, i).inner.lock().unwrap();
    let Some(w) = inner.java_wait.as_mut() else { return };
    let mut first = None;
    if let Some(names) = reload_names(text) {
        if !w.reloaded {
            w.reloaded = true;
            first = Some(format!("자바 교체 · {names} · {:.1}초", w.since.elapsed().as_secs_f64()));
        }
    } else if let Some(bean) = bean_name(text) {
        if !w.beans.iter().any(|b| b == bean) {
            w.beans.push(bean.to_string());
        }
    } else if !w.reloaded || (!is_agent && w.beans.is_empty()) {
        return;
    }
    w.last = Instant::now();
    drop(inner);
    if let Some(msg) = first {
        log(app, Some(i), "app", "hot", msg);
    }
}

/// 컴파일이 끝난 뒤 자바 반영의 끝을 지켜본다. 빈을 다시 등록했으면 조용해진 뒤 「Spring 반영」 을 한 줄 남긴다.
fn watch_java(app: AppHandle, i: usize, id: u64) {
    thread::spawn(move || loop {
        thread::sleep(Duration::from_millis(300));
        let r = rt(&app, i);
        let mut inner = r.inner.lock().unwrap();
        let Some(w) = inner.java_wait.as_ref().filter(|w| w.id == id) else { return };
        // 2026.10.29 : 교체 뒤 빈 다시 등록까지 7~10초 빈 줄 없이 지나기도 했다 (같은 날 로그). 빈을 받은 뒤에는 3초 조용하면 끝으로 본다.
        let quiet = w.last.elapsed();
        let msg = if !w.beans.is_empty() {
            if quiet < Duration::from_secs(3) {
                continue;
            }
            let secs = (w.last - w.since).as_secs_f64();
            Some(format!("Spring 반영 · {} · 빈 다시 등록 · URL 매핑 새로 만듦 · {secs:.1}초", some_names(&w.beans)))
        } else if w.reloaded && quiet > Duration::from_secs(20) {
            None // 빈이 아닌 클래스(VO · 유틸)는 교체로 끝이다
        } else if !w.reloaded && w.since.elapsed() > Duration::from_secs(60) {
            None
        } else {
            continue;
        };
        inner.java_wait = None;
        drop(inner);
        if let Some(msg) = msg {
            log(&app, Some(i), "app", "hot", msg);
        }
        return;
    });
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
    // 교체 실패는 「HOTSWAP AGENT」 머리 없이 자바 예외 줄로만 나온다 (Exception in thread … Unable to redefine classes)
    if is_redefine_failure(t) {
        return "err";
    }
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
                if let Some(names) = reload_names(&text) {
                    *r.last_reload.lock().unwrap() = names;
                }
                note_java_line(&app, i, &text);
                if is_redefine_failure(&text) && text.contains("Unable to redefine classes") {
                    let names = r.last_reload.lock().unwrap().clone();
                    let what = if names.is_empty() { "자바 클래스".to_string() } else { names };
                    add_pending(&app, i, what.clone(), "구조가 바뀌어 실행 중인 앱에 넣지 못함 (안쪽 · 익명 클래스나 상속이 바뀜)");
                    log(&app, Some(i), "app", "err", format!("{what} 을 실행 중인 앱에 넣지 못했습니다. 재시작해야 반영됩니다."));
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
// 엔진 Catalina · 호스트 localhost 아래 작업 폴더 이름은 접속 경로를 따른다 — "" 는 ROOT, "/a/b" 는 a#b (톰캣 ContextName 규칙).
// 2026.10.29 : 접속 경로를 서버 설정으로 옮기면서 ROOT 고정을 풀었다. 고정이면 /shop 앱의 세션 파일을 찾지 못해 로그인 유지가 조용히 꺼진다.
fn session_file(base: &str, context_path: &str) -> PathBuf {
    let dir = context_path.trim_start_matches('/').replace('/', "#");
    let dir = if dir.is_empty() { "ROOT".to_string() } else { dir };
    Path::new(base).join("work/Catalina/localhost").join(dir).join("SESSIONS.ser")
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
    let file = session_file(&sv.catalina_base, &sv.context_path);
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
fn report_session_save(app: &AppHandle, i: usize, sv: &ServerCfg, since_ms: u64) {
    let file = session_file(&sv.catalina_base, &sv.context_path);
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
    let (phases, opts, marks, java, pid) = {
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
        let mut m = inner.marks.clone();
        m.sort();
        (inner.phases.clone(), inner.mode.clone(), m, inner.java.clone(), inner.child.as_ref().map(|c| c.id()))
    };
    report_session_restore(app, i);
    emit_state(app);
    // jstat 이 붙는 데 0.5초쯤 걸린다 — 이 함수는 톰캣 출력을 읽는 스레드에서 돌므로 따로 띄워 출력이 밀리지 않게 한다
    let app = app.clone();
    thread::spawn(move || {
        let app = &app;
        let jvm = pid.and_then(|pid| jvm_stats(&java, pid));
        let total: u64 = phases.iter().map(|p| p.ms).sum();
        let mut detail: Vec<String> = phases.iter().map(|p| format!("{} {:.1}초", p.label, p.ms as f64 / 1000.0)).collect();
        if let Some(j) = &jvm {
            detail.push(format!("GC {:.1}초 ({}회) · 클래스 {}개 적재 {:.1}초", j.gc_ms as f64 / 1000.0, j.gc_count, j.classes, j.class_ms as f64 / 1000.0));
        }
        log(app, Some(i), "app", "ok", format!("기동 {:.1}초 · {} · {}", total as f64 / 1000.0, detail.join(" · "), opts));
        // 옵션을 바꿔 가며 비교할 수 있게 기동마다 한 줄씩 쌓는다
        let (_, sv) = cfgs(app, i);
        let csv = Path::new(&sv.catalina_base).join("logs/startup-history.csv");
        let head: Vec<&str> = PHASE_MARKS.iter().map(|(_, l)| *l).collect();
        let head = format!("시각(ms),옵션,합계(ms),{},{}", head.join(","), JVM_COLS.join(","));
        // 2026.10.29 : GC · 클래스 열을 더했다 — 열 머리가 다른 옛 파일은 옆으로 옮기고 새로 쓴다 (한 파일에 열 수가 다른 줄이 섞이지 않게)
        let old_head = fs::read_to_string(&csv).ok().and_then(|t| t.lines().next().map(str::to_string));
        if old_head.as_deref().map_or(false, |h| h != head) {
            let prev = csv.with_file_name("startup-history.prev.csv");
            let _ = fs::remove_file(&prev);
            let _ = fs::rename(&csv, &prev);
        }
        let new_file = !csv.exists();
        if let Ok(mut f) = OpenOptions::new().create(true).append(true).open(&csv) {
            if new_file {
                let _ = writeln!(f, "{head}");
            }
            let mut cols = vec![String::new(); PHASE_MARKS.len()];
            for (n, (k, _)) in marks.iter().enumerate() {
                cols[*k] = phases.get(n).map(|p| p.ms.to_string()).unwrap_or_default();
            }
            let jcols = jvm.map(|j| [j.gc_ms, j.gc_count, j.classes, j.class_ms].map(|v| v.to_string()).join(",")).unwrap_or_else(|| ",,,".into());
            let _ = writeln!(f, "{now},{},{total},{},{jcols}", opts.replace(',', " "), cols.join(","));
        }
    });
}

// 기동 기록 CSV 의 JVM 열 — jvm_stats 의 순서와 같다
const JVM_COLS: [&str; 4] = ["GC(ms)", "GC 횟수", "클래스 수", "클래스 적재(ms)"];

/// 기동이 끝난 순간 JVM 의 누적 수치 — GC 에 쓴 시간 · 횟수, 읽은 클래스 수 · 그 시간
struct JvmStats {
    gc_ms: u64,
    gc_count: u64,
    classes: u64,
    class_ms: u64,
}

// 띄운 java 와 같은 JDK 의 도구 (jstat · jstack). JRE 의 java(…/jre/bin/java.exe) 면 그 위 JDK 의 bin 을 본다. JRE 만 있으면 없다
fn jdk_tool(java: &str, name: &str) -> Option<PathBuf> {
    let bin = Path::new(java).parent()?;
    [Some(bin.join(name)), bin.parent().and_then(Path::parent).map(|jdk| jdk.join("bin").join(name))].into_iter().flatten().find(|p| p.is_file())
}

// 도구를 띄워 표준 출력을 받는다. 멈춘 JVM 에는 붙지 못하고 기다리기만 하므로 시간을 넘기면 끊는다
fn tool_output(exe: &Path, args: &[&str], limit: Duration) -> Option<String> {
    let mut child = Command::new(exe).args(args).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::null()).creation_flags(CREATE_NO_WINDOW).spawn().ok()?;
    let mut out = child.stdout.take()?;
    let reader = thread::spawn(move || {
        let mut b = Vec::new();
        let _ = out.read_to_end(&mut b);
        b
    });
    let until = Instant::now() + limit;
    loop {
        match child.try_wait() {
            Ok(Some(st)) => {
                let b = reader.join().ok()?;
                return st.success().then(|| decode(&b));
            }
            Ok(None) if Instant::now() < until => thread::sleep(Duration::from_millis(50)),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
        }
    }
}

fn jvm_stats(java: &str, pid: u32) -> Option<JvmStats> {
    let jstat = jdk_tool(java, "jstat.exe")?;
    let pid = pid.to_string();
    let gc = tool_output(&jstat, &["-gcutil", &pid], Duration::from_secs(5))?;
    let cls = tool_output(&jstat, &["-class", &pid], Duration::from_secs(5))?;
    parse_jstat(&gc, &cls)
}

// jstat -gcutil : 「S0 S1 E O M CCS YGC YGCT FGC FGCT GCT」 · -class : 「Loaded Bytes Unloaded Bytes Time」 (시간은 초)
fn parse_jstat(gc: &str, cls: &str) -> Option<JvmStats> {
    let rows = |t: &str| -> Option<(Vec<String>, Vec<String>)> {
        let mut l = t.lines().filter(|l| !l.trim().is_empty());
        let h = l.next()?.split_whitespace().map(str::to_string).collect();
        let v = l.next()?.split_whitespace().map(str::to_string).collect();
        Some((h, v))
    };
    let (gh, gv) = rows(gc)?;
    let g = |name: &str| gh.iter().position(|h| h == name).and_then(|k| gv.get(k)).and_then(|v| v.parse::<f64>().ok());
    let (_, cv) = rows(cls)?;
    let ms = |s: f64| (s * 1000.0).round() as u64;
    Some(JvmStats {
        gc_ms: ms(g("GCT")?),
        gc_count: (g("YGC")? + g("FGC")?) as u64,
        classes: cv.first()?.parse().ok()?,
        class_ms: ms(cv.last()?.parse().ok()?),
    })
}

/* ---------- 기동 표본 (「설정 › 기동 › 기동 표본 남기기」) ---------- */

// 「Spring 루트 설정」 · 「Spring MVC」 처럼 로그 한 줄 없이 10초 넘게 지나가는 구간이 있다 (2026-10-29 · 10.1초 · 13.5초).
// 켜는 동안 2초마다 jstack 으로 기동 스레드의 호출 스택을 떠서, 어디에 가장 자주 머물렀는지 세어 파일로 남긴다.
const SAMPLE_EVERY: Duration = Duration::from_secs(2);
// JDK 안쪽 프레임은 건너뛰고 그것을 부른 쪽으로 센다 — 「ZipFile.read」 보다 「그것을 부른 jar 스캐너」가 단서다
const JDK_FRAMES: [&str; 5] = ["java.", "javax.", "sun.", "jdk.", "com.sun."];
const INCLUSIVE_DEPTH: usize = 12;

fn start_sampler(app: AppHandle, i: usize, generation: u64, java: String, pid: u32, base: String) {
    thread::spawn(move || {
        let app = &app;
        let Some(jstack) = jdk_tool(&java, "jstack.exe") else {
            log(app, Some(i), "app", "warn", format!("기동 표본을 뜨지 못합니다 · jstack.exe 가 없습니다 (JRE 로 띄움): {java}"));
            return;
        };
        let r = rt(app, i);
        let pid_s = pid.to_string();
        let mut samples: Vec<(u64, Vec<Vec<String>>)> = Vec::new();
        loop {
            thread::sleep(SAMPLE_EVERY);
            {
                let inner = r.inner.lock().unwrap();
                if inner.generation != generation || inner.status != Status::Starting {
                    break;
                }
            }
            let at = now_ms();
            if let Some(text) = tool_output(&jstack, &[&pid_s], Duration::from_secs(10)) {
                samples.push((at, startup_stacks(&text)));
            }
        }
        if samples.is_empty() {
            return;
        }
        let (marks, name) = {
            let mut m = r.inner.lock().unwrap().marks.clone();
            m.sort_by_key(|x| x.1);
            (m, cfgs(app, i).1.name)
        };
        let path = Path::new(&base).join("logs").join(format!("startup-profile-{}.txt", now_ms()));
        let report = profile_report(&name, &samples, &marks);
        match fs::write(&path, report) {
            Ok(_) => log(app, Some(i), "app", "", format!("기동 표본 {}개를 남겼습니다: {}", samples.len(), p(&path))),
            Err(e) => log(app, Some(i), "app", "warn", format!("기동 표본을 쓰지 못했습니다: {e}")),
        }
    });
}

// jstack 출력에서 기동 스레드(localhost-startStop-*)의 JDK 바깥 프레임만 — 그 스레드가 없으면(웹앱 준비 전) main.
// main 은 웹앱이 뜨는 동안 그 스레드를 기다리기만 하므로 함께 세면 「기다림」이 절반을 차지한다.
fn startup_stacks(text: &str) -> Vec<Vec<String>> {
    let mut threads: Vec<(String, Vec<String>)> = Vec::new();
    for line in text.lines() {
        if let Some(rest) = line.strip_prefix('"') {
            threads.push((rest.split('"').next().unwrap_or("").to_string(), Vec::new()));
        } else if let Some(f) = line.trim_start().strip_prefix("at ") {
            if let Some(t) = threads.last_mut() {
                let frame = f.split('(').next().unwrap_or(f).to_string();
                if !JDK_FRAMES.iter().any(|j| frame.starts_with(j)) {
                    t.1.push(frame);
                }
            }
        }
    }
    let pick = |want: &dyn Fn(&str) -> bool| threads.iter().filter(|t| want(&t.0) && !t.1.is_empty()).map(|t| t.1.clone()).collect::<Vec<_>>();
    let work = pick(&|n| n.starts_with("localhost-startStop"));
    if work.is_empty() { pick(&|n| n == "main") } else { work }
}

fn profile_report(name: &str, samples: &[(u64, Vec<Vec<String>>)], marks: &[(usize, u64)]) -> String {
    fn top(counts: Vec<(String, usize)>, n: usize, of: usize) -> String {
        let mut v = counts;
        v.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
        v.iter().take(n).map(|(f, c)| format!("  {c:>4}회 ({:>3}%)  {f}\n", c * 100 / of.max(1))).collect()
    }
    fn bump(m: &mut Vec<(String, usize)>, f: &str) {
        match m.iter_mut().find(|x| x.0 == f) {
            Some(x) => x.1 += 1,
            None => m.push((f.to_string(), 1)),
        }
    }
    // 표본 시각 다음에 처음 잡힌 표지가 그 표본의 단계다 (단계 이름은 그 표지에서 끝나는 구간)
    let phase_of = |at: u64| marks.iter().find(|(_, t)| *t >= at).map_or("기동 뒤", |(k, _)| PHASE_MARKS[*k].1);
    let stacks: usize = samples.iter().map(|s| s.1.len()).sum();
    let (mut self_c, mut incl) = (Vec::new(), Vec::new());
    let mut phases: Vec<(&str, usize, Vec<(String, usize)>)> = Vec::new();
    for (at, list) in samples {
        let ph = phase_of(*at);
        if !phases.iter().any(|p| p.0 == ph) {
            phases.push((ph, 0, Vec::new()));
        }
        let slot = phases.iter_mut().find(|p| p.0 == ph).unwrap();
        for st in list {
            bump(&mut self_c, &st[0]);
            bump(&mut slot.2, &st[0]);
            slot.1 += 1;
            let mut seen: Vec<&str> = Vec::new();
            for f in st.iter().take(INCLUSIVE_DEPTH) {
                if !seen.contains(&f.as_str()) {
                    seen.push(f);
                    bump(&mut incl, f);
                }
            }
        }
    }
    let mut out = format!(
        "기동 표본 · {name}\n표본 {}개 · {}초마다 · 기동 스레드 스택 {stacks}개 (localhost-startStop-*, 없으면 main)\n\
         jstack 이 붙는 동안 JVM 이 잠깐 멈추므로, 이 기동의 시간은 표본 없이 뜬 기동과 견주지 않는다.\n\n",
        samples.len(),
        SAMPLE_EVERY.as_secs()
    );
    out.push_str("[맨 위] 그 자리에서 머문 곳 (JDK 바깥 첫 프레임)\n");
    out.push_str(&top(self_c, 20, stacks));
    out.push_str(&format!("\n[포함] 그 아래 호출까지 포함 (JDK 바깥 프레임 위 {INCLUSIVE_DEPTH}단)\n"));
    out.push_str(&top(incl, 20, stacks));
    out.push_str("\n[단계별 맨 위]\n");
    for (ph, n, c) in phases {
        out.push_str(&format!("{ph} · 스택 {n}개\n"));
        out.push_str(&top(c, 8, n));
    }
    out
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
        for i in 0..rt_count(&app) {
            let r = rt(&app, i);
            // 콘솔 로그 버퍼를 비운다 — 실행기가 죽어도 잃는 것은 길어야 이 주기만큼이다
            if let Some(f) = r.console.lock().unwrap().as_mut() {
                let _ = f.flush();
            }
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
            // 실행기가 쓰는 중이면 반쯤 쓴 상태를 읽지 않는다 — 끝나면 vcs_done 이 표지를 맞추고 한 번만 알린다
            if r.vcs_busy.load(Ordering::SeqCst) {
                continue;
            }
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
        ("contextPath", sv.context_path.clone()),
        // 0 이면 톰캣이 파라미터를 하나도 받지 않는다 — 손으로 고친 json 이 0 이어도 기본으로 띄운다
        ("maxParameterCount", if sv.max_parameter_count == 0 { DEFAULT_MAX_PARAMS } else { sv.max_parameter_count }.to_string()),
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
// 이미 있으면 덮지 않는다 — 사용자가 그 서버의 conf 만 고쳐 쓸 수 있게.
// 맨 위 파일만 복사한다 — conf/Catalina/ 아래(서버별 JNDI 설정 · write_naming)는 첫 서버의 것이 다른 서버로 퍼지면 안 된다.
// 이미 있는 server.xml 이 옛 원본 그대로면 새 원본으로 바꾼다 (upgrade_server_xml). 돌려주는 값은 기동 로그에 남길 경고다.
fn ensure_base(sv: &ServerCfg) -> Result<Option<String>, String> {
    let base = Path::new(&sv.catalina_base);
    let conf = base.join("conf");
    let tpl = home_dir().join("base/conf");
    let mut warning = None;
    if conf.join("server.xml").exists() {
        // 첫 서버는 실행기 폴더의 base 를 그대로 쓴다 — 원본 자신이다
        if !same_dir(&conf, &tpl) {
            if let Ok(tpl_xml) = fs::read_to_string(tpl.join("server.xml")) {
                warning = upgrade_server_xml(&conf.join("server.xml"), &tpl_xml)?;
            }
        }
    } else {
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
    Ok(warning)
}

fn same_dir(a: &Path, b: &Path) -> bool {
    let norm = |x: &Path| fs::canonicalize(x).map(|c| p(&c)).unwrap_or_else(|_| p(x)).trim_end_matches('/').to_lowercase();
    norm(a) == norm(b)
}

// 서버 설정 값으로 받게 된 속성 — (옛 원본에 박혀 있던 글자, 지금 원본의 글자, 설정 칸 이름)
const SERVER_XML_PROPS: [(&str, &str, &str); 2] = [
    (r#"<Context path="""#, r#"<Context path="${tomcatrun.contextPath}""#, "접속 경로"),
    (r#"maxParameterCount="1000""#, r#"maxParameterCount="${tomcatrun.maxParameterCount}""#, "파라미터 수 상한"),
];

// 주석과 공백을 뺀 본문 — 원본 머리말 설명 · 줄바꿈만 바뀐 것은 같은 파일로 본다
fn xml_body(xml: &str) -> String {
    let mut out = String::new();
    let mut rest = xml;
    while let Some(at) = rest.find("<!--") {
        out.push_str(&rest[..at]);
        rest = rest[at..].find("-->").map_or("", |end| &rest[at + end + 3..]);
    }
    out.push_str(rest);
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// 서버를 더할 때 복사해 둔 server.xml 을 새 원본으로 옮긴다 (2026.10.29 · 접속 경로 · maxParameterCount 를 서버 설정으로 옮기며 생겼다).
/// ensure_base 는 이미 있는 conf 를 덮지 않아서, 그대로 두면 더한 서버는 설정 화면에서 두 값을 바꿔도 톰캣에 들어가지 않는다.
///   옛 박힌 값을 ${…} 로 바꾼 본문이 지금 원본과 같다   → 손대지 않은 복사본이다. server.xml.bak 을 남기고 원본으로 바꾼다
///   다르다 (사용자가 고쳐 쓴 파일)                       → 건드리지 않고 경고만 돌려준다
fn upgrade_server_xml(file: &Path, tpl_xml: &str) -> Result<Option<String>, String> {
    let Ok(cur) = fs::read_to_string(file) else { return Ok(None) };
    let missing: Vec<_> = SERVER_XML_PROPS.iter().filter(|(_, new, _)| !cur.contains(new)).collect();
    if missing.is_empty() {
        return Ok(None);
    }
    let mut moved = cur.clone();
    for (old, new, _) in SERVER_XML_PROPS {
        moved = moved.replace(old, new);
    }
    if xml_body(&moved) == xml_body(tpl_xml) {
        let bak = file.with_extension("xml.bak");
        fs::copy(file, &bak).map_err(|e| format!("톰캣 설정 사본을 남기지 못했습니다 ({}): {e}", p(&bak)))?;
        fs::write(file, tpl_xml).map_err(|e| format!("톰캣 설정을 새 원본으로 바꾸지 못했습니다 ({}): {e}", p(file)))?;
        return Ok(None);
    }
    Ok(Some(format!(
        "{} 가 실행기 원본과 달라(직접 고쳤거나 더 옛 판의 복사본) 바꾸지 않았습니다. 「설정 › 서버」의 {} 가 이 서버에 들어가지 않습니다 — 쓰려면 그 파일에 {} 를 적거나, 파일을 지우고 다시 시작해 원본을 복사하게 하세요.",
        p(file),
        missing.iter().map(|m| m.2).collect::<Vec<_>>().join(" · "),
        missing.iter().map(|m| m.1.trim_start_matches("<Context ")).collect::<Vec<_>>().join(" · ")
    )))
}

/* ---------- 이클립스 JNDI 설정 (서버별) ---------- */

// 이클립스 server.xml · context.xml 의 <Resource> 를 옮겨 적는 곳 — 톰캣이 conf/context.xml 다음에 모든 Context 에 덧붙인다.
// 실행기 server.xml · context.xml 은 git 에 들어 있고 다른 서버의 템플릿이라 DB 계정 · 비밀번호를 넣지 않는다.
// 이 파일은 서버마다 따로 있고 .gitignore(/base/conf/Catalina/ · /servers/)에 들어 있다.
const NAMING_MARK: &str = "tomcatrun 이 이클립스 설정에서 옮긴 JNDI";

fn naming_file(base: &str) -> PathBuf {
    Path::new(base).join("conf/Catalina/localhost/context.xml.default")
}

fn xml_attr(v: &str) -> String {
    v.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

fn naming_xml(entries: &[&eclipse::NamingEntry], source: &str) -> String {
    let mut s = format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<!--\n  {NAMING_MARK} — {}\n  이클립스에서 다시 가져오며 JNDI 를 옮기면 이 파일을 갈아 쓴다. 손으로 고친 것은 그때 사라진다.\n  DB 계정 · 비밀번호가 들어 있다. 이 폴더 밖으로 복사하지 않는다.\n-->\n<Context>\n",
        source.replace("--", "- -")
    );
    for e in entries {
        let attrs: Vec<String> = e.attrs.iter().map(|(k, v)| format!("{k}=\"{}\"", xml_attr(v))).collect();
        s.push_str(&format!("  <{} {}/>\n", e.kind, attrs.join(" ")));
    }
    s.push_str("</Context>\n");
    s
}

/// 옮길 수 있는 항목을 서버의 context.xml.default 에 쓴다. 실행기가 만들지 않은 파일이 있으면 .bak 으로 남긴다. 쓴 수를 돌려준다.
fn write_naming(base: &str, entries: &[eclipse::NamingEntry], source: &str) -> Result<usize, String> {
    let list: Vec<&eclipse::NamingEntry> = entries.iter().filter(|e| e.movable()).collect();
    if list.is_empty() {
        return Ok(0);
    }
    let file = naming_file(base);
    if let Some(dir) = file.parent() {
        fs::create_dir_all(dir).map_err(|e| format!("톰캣 설정 폴더를 만들지 못했습니다 ({}): {e}", p(dir)))?;
    }
    if let Ok(old) = fs::read_to_string(&file) {
        if !old.contains(NAMING_MARK) {
            fs::copy(&file, file.with_extension("default.bak")).map_err(|e| format!("있던 {} 을 남기지 못했습니다: {e}", p(&file)))?;
        }
    }
    fs::write(&file, naming_xml(&list, source)).map_err(|e| format!("JNDI 설정을 쓰지 못했습니다 ({}): {e}", p(&file)))?;
    Ok(list.len())
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

fn open_console_log(base: &str) -> Option<std::io::BufWriter<File>> {
    let dir = Path::new(base).join("logs");
    let _ = fs::create_dir_all(&dir);
    let cur = dir.join("console.log");
    let _ = fs::rename(&cur, dir.join("console.prev.log"));
    OpenOptions::new().create(true).write(true).truncate(true).open(cur).ok().map(std::io::BufWriter::new)
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
    if let Some(w) = ensure_base(&sv).map_err(|e| format!("{name} {e}"))? {
        // 기동마다 쌓이지 않게 실행기를 켠 동안 파일마다 한 번만
        static WARNED: Mutex<Vec<String>> = Mutex::new(Vec::new());
        let mut warned = WARNED.lock().unwrap();
        if !warned.contains(&sv.catalina_base) {
            warned.push(sv.catalina_base.clone());
            log(app, Some(i), "app", "warn", format!("{name} {w}"));
        }
    }
    let java = launch_java(&cfg, &sv);
    let major = java_major(&java);
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
        inner.java_wait = None;
        let heap =format!("힙 {}MB", cfg.heap_mb);
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
        if cfg.profile_start {
            mode.push("표본");
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
        // 끈 채 인코딩 · -parameters 를 바꿨으면 톰캣을 띄우기 전에 비우고 다시 만든다 — 아직 아무것도 클래스를 쥐고 있지 않다
        let opts_changed = read_stamp(&sv.classes_dir) != 0 && read_compile_opts(&sv.classes_dir) != compile_opts(&sv);
        if opts_changed {
            let _ = fs::remove_dir_all(&sv.classes_dir);
        }
        if !Path::new(&sv.classes_dir).join(STAMP_FILE).exists() {
            if opts_changed {
                log(app, Some(i), "app", "", format!("컴파일 옵션이 바뀌어({}) 컴파일 결과를 비우고 전체 컴파일부터 합니다.", opts_label(&sv)));
            } else {
                log(app, Some(i), "app", "", "컴파일한 클래스가 없어 전체 컴파일부터 합니다.");
            }
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
            // 2026.10.29 : JSP 클래스 로더(JasperLoader)는 HotswapAgent 가 맡지 않게 한다 — JSP 는 Jasper 가 스스로 다시 컴파일하는데,
            // 화면을 처음 열 때마다 로더가 하나씩 생겨 그때마다 폴더 다섯 곳 감시를 다시 걸었다 (콘솔에 「Registering directory」 다섯 줄씩)
            let props = Path::new(base).join("temp/hotswap-agent.properties");
            let _ = fs::create_dir_all(props.parent().unwrap());
            let extra = match fs::write(&props, "# tomcatrun 이 기동마다 쓴다 (손으로 고쳐도 다음 기동에 덮인다)\nexcludedClassLoaderPatterns=org.apache.jasper.*\n") {
                Ok(_) => format!(",propertiesFilePath={}", p(&props)),
                Err(_) => String::new(),
            };
            args.push(format!("-javaagent:{}=autoHotswap=true,{}{extra}", cfg.hotswap_agent, disabled.join(",")));
            // 서버 JVM 인자에 인증서 목록을 직접 준 사람은 그것을 따른다
            if !sv.jvm_args.iter().any(|a| a.starts_with("-Djavax.net.ssl.trustStore=")) {
                match merged_trust_store(&java, base) {
                    Ok((store, added)) => {
                        log(app, Some(i), "app", "", format!("인증서 목록: DCEVM 기본에 Windows 루트 인증서 {added}개를 더했습니다."));
                        args.push(format!("-Djavax.net.ssl.trustStore={store}"));
                        args.push("-Djavax.net.ssl.trustStorePassword=changeit".into());
                    }
                    Err(e) => log(app, Some(i), "app", "warn", format!("인증서 목록을 만들지 못해 DCEVM 기본 목록으로 뜹니다. 외부 HTTPS 호출이 PKIX 오류로 막힐 수 있습니다 · {e}")),
                }
            }
        }
        if cfg.debug {
            args.push(format!("-agentlib:jdwp=transport=dt_socket,server=y,suspend=n,address={}", sv.debug_port));
        }
        if cfg.fast_jvm {
            // 2026.10.29 : 자바 13 부터 -Xverify:none 은 쓰지 말라는 경고를 남긴다 (검증 생략은 그대로 된다) — 로그가 어지럽지 않게 뺀다
            args.extend(FAST_JVM_ARGS.iter().filter(|a| major < 13 || **a != "-Xverify:none").map(|s| s.to_string()));
        }
        // 2026.10.29 : 자바 9 이상 — 톰캣 8.5 의 catalina.bat 가 주는 것과 같다. 없으면 톰캣이 메모리 누수 막기 · RMI 정리에서 경고를 남긴다
        if major >= 9 {
            args.extend(JDK9_ARGS.iter().map(|s| s.to_string()));
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
        ]);
        // 2026.10.29 : 자바 9 이상은 endorsed 를 받지 않는다 — 주면 「-Djava.endorsed.dirs is not supported」 로 JVM 이 뜨지 않는다
        if major < 9 {
            args.push(format!("-Djava.endorsed.dirs={home}/endorsed"));
        }
        args.extend([
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
            inner.session_had = if cfg.keep_session { fs::metadata(session_file(base, &sv.context_path)).ok().map(|m| m.len()) } else { None };
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
        let pid = child.id();
        r.inner.lock().unwrap().child = Some(child);
        emit_state(app);
        if cfg.profile_start {
            start_sampler(app.clone(), i, generation, java.clone(), pid, base.clone());
        }
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
                    report_session_save(app, i, &sv, asked_ms);
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
    // 인코딩 · -parameters 가 지난 컴파일과 다르면 전체를 다시 — 그러지 않으면 옛 옵션으로 만든 클래스가 남는다
    let opts = compile_opts(&sv);
    let opts_changed = stamp != 0 && read_compile_opts(&sv.classes_dir) != opts;
    if opts_changed {
        log(app, Some(i), "compile", "", format!("컴파일 옵션이 바뀌어({}) 전체를 다시 컴파일합니다.", opts_label(&sv)));
    }
    let full = full || stamp == 0 || opts_changed;
    let begin_ms = now_ms();
    let began = Instant::now();

    let src = Path::new(&sv.project).join("src/main/java");
    let mut files = Vec::new();
    // 2026.10.29 : 저장마다 소스 폴더 전체를 훑어 바뀐 파일을 찾는다 — 그 시간을 완료 줄에 따로 보인다 (크면 저장 감시가 준 경로를 먼저 쓰는 쪽으로)
    collect_java(&src, if full { 0 } else { stamp }, &mut files);
    let find_ms = began.elapsed().as_millis();
    if files.is_empty() {
        log(app, Some(i), "compile", "", format!("바뀐 자바 파일이 없습니다 (찾기 {find_ms}ms)."));
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

    let argfile = Path::new(&sv.catalina_base).join("temp/javac-files.txt");
    let _ = fs::create_dir_all(argfile.parent().unwrap());
    let cp = build_cp(&cfg, &sv);
    // 핫리로드로 켜져 있으면 교체 → Spring 반영까지 지켜본다. 교체 줄이 컴파일 완료보다 먼저 올 수 있어 컴파일 전에 건다.
    let wait_id = {
        let mut inner = r.inner.lock().unwrap();
        (inner.status == Status::Running && inner.mode.starts_with("핫리로드")).then(|| {
            let id = now_ms();
            inner.java_wait = Some(JavaWait { id, since: began, reloaded: false, beans: Vec::new(), last: Instant::now() });
            id
        })
    };
    log(
        app,
        Some(i),
        "compile",
        "",
        if full { format!("전체 컴파일 시작 · {}개 파일", files.len()) } else { format!("컴파일 시작 · {}", names(&files)) },
    );
    let on_diag = |t: String| {
        let kind = if t.contains(": error:") || t.contains(": 오류:") { "err" } else { classify(&t) };
        log(app, Some(i), "compile", kind, t);
    };
    let compile = |list: &[PathBuf]| -> (bool, usize, String) {
        // javac 인자 파일 안에서 역슬래시는 이스케이프로 읽히므로 / 로 적는다
        let _ = fs::write(&argfile, list.iter().map(|f| format!("\"{}\"\n", p(f))).collect::<String>());
        // 띄워 둔 상주 컴파일러가 먼저다. 없거나 응답이 없으면 javac 를 새로 띄워 같은 일을 한다.
        let request = compile_request(&sv.classes_dir, &cp, &argfile, &sv);
        match daemon_call(app, &request, &on_diag) {
            Some((ok, errors)) => (ok, errors, format!("상주 {}", app.state::<Shared>().daemon_kind.lock().unwrap())),
            None => {
                let (ok, errors) = javac_process(app, i, &cfg, &sv, &cp, &argfile);
                (ok, errors, "javac 프로세스".to_string())
            }
        }
    };
    let rels = |list: &[PathBuf]| list.iter().map(|f| f.strip_prefix(&src).unwrap_or(f).to_string_lossy().replace('\\', "/")).collect::<Vec<_>>();
    let mut before = if full { Default::default() } else { classapi::snapshot(&classes, &rels(&files)) };
    let (mut ok, mut errors, via) = compile(&files);
    // 상수 값 · 메서드 시그니처가 바뀌었으면 쓰는 소스도 컴파일한다 (classapi.rs 머리말).
    // 그 소스가 가진 상수가 바뀐 상수로 셈한 것일 수 있어 몇 번 거듭한다. 전체 컴파일이면 이미 다 했다.
    let mut compiled = files.clone();
    let mut round = files.clone();
    for _ in 0..3 {
        if !ok || full {
            break;
        }
        let shapes = classapi::changed(&before, &classapi::snapshot(&classes, &rels(&round)));
        let skip: HashSet<PathBuf> = compiled.iter().cloned().collect();
        let users = classapi::users(&src, &shapes, &skip);
        if users.is_empty() {
            break;
        }
        // 「컴파일」 로 시작해야 반영 기록에도 남는다 (ui 의 isEvent)
        log(app, Some(i), "compile", "", format!("컴파일 더함 · {} 의 상수 · 메서드 시그니처가 바뀌어 쓰는 곳 {}개 · {}", some_names(&shapes), users.len(), names(&users)));
        before = classapi::snapshot(&classes, &rels(&users));
        let (o, e, _) = compile(&users);
        (ok, errors) = (o, errors + e);
        compiled.extend(users.iter().cloned());
        round = users;
    }
    let files = compiled;
    if let Some(id) = wait_id {
        let mut inner = r.inner.lock().unwrap();
        if !ok && inner.java_wait.as_ref().map_or(false, |w| w.id == id) {
            inner.java_wait = None;
        } else if ok {
            drop(inner);
            watch_java(app.clone(), i, id);
        }
    }
    if ok {
        // 시작 시각을 남긴다 — 컴파일하는 동안 고친 파일은 다음에 다시 잡힌다
        let _ = fs::write(classes.join(STAMP_FILE), begin_ms.to_string());
        let _ = fs::write(classes.join(COMPILE_OPTS_FILE), &opts);
        // 켜진 채 옵션이 바뀌었으면(설정 파일을 손으로 고친 경우 — 화면에서는 끈 뒤에만 바꾼다) 옛 클래스가 앱에 올라가 있다
        if opts_changed && !stopped {
            add_pending(app, i, "자바 컴파일 옵션".into(), "인코딩 · -parameters 가 바뀜 · 재시작해야 모두 들어감");
        }
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
        log(app, Some(i), "compile", "ok", format!("컴파일 완료 · {}개 · {secs:.1}초 (바뀐 파일 찾기 {find_ms}ms · {via}){tail}", result.files));
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

fn javac_encoding(sv: &ServerCfg) -> &str {
    if sv.encoding.trim().is_empty() { "UTF-8" } else { sv.encoding.trim() }
}

// 컴파일 결과 폴더에 남기는 옵션 한 줄 — 「인코딩 탭 -parameters」. 견줄 때 쓰므로 인코딩 이름은 서버 설정 그대로 (MS949 와 CP949 를 바꿔 적어도 한 번 다시 만든다)
fn compile_opts(sv: &ServerCfg) -> String {
    format!("{}\t{}", javac_encoding(sv), sv.parameters)
}

// 옵션 파일이 없는 폴더(이 기능 전에 만든 것)는 그때 늘 쓰던 UTF-8 · 끔으로 본다 — 업데이트하자마자 전체를 다시 만들지 않게
fn read_compile_opts(classes_dir: &str) -> String {
    fs::read_to_string(Path::new(classes_dir).join(COMPILE_OPTS_FILE)).map(|t| t.trim().to_string()).unwrap_or_else(|_| "UTF-8\tfalse".into())
}

fn opts_label(sv: &ServerCfg) -> String {
    format!("인코딩 {}{}", javac_encoding(sv), if sv.parameters { " · -parameters" } else { "" })
}

// 상주 컴파일러에 보내는 한 줄 — 컴파일러 하나를 여러 서버가 같이 쓰므로 옵션은 기동 인자가 아니라 요청마다 싣는다
fn compile_request(out_dir: &str, cp: &str, argfile: &Path, sv: &ServerCfg) -> String {
    format!("COMPILE\t{out_dir}\t{cp}\t{}\t{}\t{}", p(argfile), javac_encoding(sv), sv.parameters)
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
    // -encoding · -parameters 는 서버 설정 (상주 컴파일러에 싣는 것과 같다 — compile_request)
    let out = Command::new(&cfg.javac)
        .args(["-nowarn", "-g", "-encoding", javac_encoding(sv), "-source", "1.8", "-target", "1.8", "-implicit:none", "-sourcepath", ""])
        .args(sv.parameters.then_some("-parameters"))
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
    let began = Instant::now();
    loop {
        thread::sleep(Duration::from_secs(2));
        let n = rt_count(app);
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
        // 결과는 버리지만 인코딩은 맞춘다 — 다르면 ECJ 가 한글 줄마다 진단을 낸다
        if daemon_call(app, &compile_request(&p(&tmp), &build_cp(&cfg, sv), &argfile, sv), |_| {}).is_none() {
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
        let count = rt_count(&app);
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

// 자바 소스를 지웠거나 옮겼다 — 컴파일 결과의 클래스도 지운다.
// 2026.10.29 : 바뀐 파일만 컴파일하므로 지운 소스의 .class 가 남아, 재시작해도 Spring 이 그 클래스를 빈으로 다시 읽었다.
// 실행 중인 앱에는 이미 올라가 있어 빼지 못한다 — 재시작 알림에 올린다.
fn removed_java(app: &AppHandle, i: usize, sv: &ServerCfg, path: &Path, running: bool) {
    let rel = rel_to(&sv.project, path);
    let Some(in_src) = rel.get("src/main/java/".len()..) else { return };
    let n = classapi::remove(Path::new(&sv.classes_dir), in_src);
    if n == 0 {
        return;
    }
    let name = in_src.rsplit('/').next().unwrap_or(in_src);
    if running {
        add_pending(app, i, rel.clone(), "자바 소스를 지움 · 실행 중인 앱에는 그 클래스가 남아 있음");
    }
    let (kind, tail) = if running { ("warn", " · 재시작해야 앱에서 빠집니다") } else { ("", "") };
    log(app, Some(i), "app", kind, format!("{name} 을 지워 컴파일 결과의 클래스 {n}개도 지웠습니다{tail}"));
}

fn apply_changes(app: &AppHandle, i: usize, paths: &[&PathBuf]) {
    let (c, sv) = cfgs(app, i);
    // resources · filters 파일이 바뀌었으면 리소스 필터링 점검을 다시 세게 한다 (무시 목록에 걸린 파일도 톰캣은 올린다)
    let res = |p: &&PathBuf| {
        let l = rel_to(&sv.project, p).to_ascii_lowercase();
        l.starts_with("src/main/") && !l.starts_with("src/main/java/") && !l.starts_with("src/main/webapp/")
    };
    if paths.iter().any(res) {
        rt(app, i).res_gen.fetch_add(1, Ordering::Relaxed);
    }
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
            Some(Change::Java) if !path.exists() => removed_java(app, i, &sv, path, running),
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

// 켤 때 · 프로젝트 폴더가 바뀌었을 때 — 감시를 모두 걷고 지금 설정으로 다시 건다 (실행기를 다시 열지 않아도 새 폴더를 감시한다)
fn start_watchers(app: &AppHandle) {
    let s = shared(app);
    let servers = s.cfg.lock().unwrap().servers.clone();
    let mut watchers = s.watchers.lock().unwrap();
    watchers.clear();
    for (i, sv) in servers.iter().enumerate() {
        *rt(app, i).live_mappers.lock().unwrap() = live_mappers(&sv.project);
        if sv.project.trim().is_empty() {
            continue;
        }
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
                watchers.push(w);
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
    redo_dedup(app, Some(i), &sv);
    true
}

// 「겹친 판 치우기」를 켠 서버는 의존 jar 를 새로 채운 뒤 같은 규칙으로 다시 치운다 — 폴더가 통째로 갈려 치운 판이 되살아나기 때문이다.
// 옛 deps.dup 는 지난 폴더에서 치운 사본이라 비우고 새로 채운다.
fn redo_dedup(app: &AppHandle, i: Option<usize>, sv: &ServerCfg) {
    if !sv.deps_dedup {
        return;
    }
    let deps = Path::new(&sv.deps_dir);
    let _ = fs::remove_dir_all(jarcheck::aside_dir(deps));
    match jarcheck::sweep(&own_lib(&sv.project), deps) {
        Ok(0) => {}
        Ok(n) => log(app, i, "deps", "ok", format!("{} 같은 jar 다른 판 {n}개를 다시 치웠습니다 (deps.dup).", sv.name)),
        Err(e) => log(app, i, "deps", "err", format!("{} 같은 jar 다른 판을 다시 치우지 못했습니다: {e}", sv.name)),
    }
}

// 치우기 · 되돌리기는 jar 를 옮기므로 톰캣이 꺼져 있고 받는 중이 아닐 때만
fn dedup_ready(app: &AppHandle, srv: usize) -> Result<ServerCfg, String> {
    check_srv(app, srv)?;
    let (r, sv) = (rt(app, srv), cfgs(app, srv).1);
    if r.inner.lock().unwrap().status != Status::Stopped {
        return Err(format!("{} 톰캣을 끈 뒤에 하세요. 실행 중에는 jar 가 잠겨 옮길 수 없습니다.", sv.name));
    }
    if r.deps_busy.load(Ordering::SeqCst) {
        return Err(format!("{} 의존 jar 를 받는 중입니다. 끝난 뒤에 하세요.", sv.name));
    }
    Ok(sv)
}

fn set_dedup(app: &AppHandle, srv: usize, on: bool) -> Result<(), String> {
    let mut c = shared(app).cfg.lock().unwrap();
    c.servers[srv].deps_dedup = on;
    save_config(&c).map_err(|e| format!("설정을 저장하지 못했습니다: {e}"))
}

#[tauri::command]
fn dedup_deps(app: AppHandle, srv: usize) -> Result<String, String> {
    let sv = dedup_ready(&app, srv)?;
    let n = jarcheck::sweep(&own_lib(&sv.project), Path::new(&sv.deps_dir))?;
    set_dedup(&app, srv, true)?;
    // 상주 컴파일러가 치운 jar 를 classpath 에 들고 있다
    stop_daemon(&app, "");
    emit_state(&app);
    let msg = if n == 0 {
        format!("{} 의존 jar 쪽에서 치울 판이 없습니다. 남은 겹침은 프로젝트 WEB-INF/lib 에서 지워야 합니다.", sv.name)
    } else {
        format!("{} 같은 jar 다른 판 {n}개를 deps.dup 로 치웠습니다 · 다음 시작부터 한 판만 올라갑니다.", sv.name)
    };
    log(&app, Some(srv), "deps", "ok", msg.clone());
    Ok(msg)
}

#[tauri::command]
fn undo_dedup(app: AppHandle, srv: usize) -> Result<String, String> {
    let sv = dedup_ready(&app, srv)?;
    let n = jarcheck::restore(Path::new(&sv.deps_dir))?;
    set_dedup(&app, srv, false)?;
    stop_daemon(&app, "");
    emit_state(&app);
    let msg = format!("{} 치운 jar {n}개를 의존 jar 폴더로 돌려놓았습니다 · 다시 받아도 치우지 않습니다.", sv.name);
    log(&app, Some(srv), "deps", "ok", msg.clone());
    Ok(msg)
}

/* ---------- 화면 명령 ---------- */

fn check_srv(app: &AppHandle, srv: usize) -> Result<(), String> {
    if srv < rt_count(app) {
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
    (0..rt_count(app))
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
    let n = rt_count(&app);
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
            "profileStart" => c.profile_start = value,
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
    for i in 0..rt_count(&app) {
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
        c.msg_cmd = paths.msg_cmd.trim().to_string();
        c.tld_scan = paths.tld_scan.split([',', '\n']).map(str::trim).filter(|x| !x.is_empty()).collect::<Vec<_>>().join(",");
        save_config(&c).map_err(|e| format!("설정을 저장하지 못했습니다: {e}"))?;
    }
    emit_state(&app);
    Ok(())
}

// 「설정 › 서버」 — 포트 · 이름 · 바로가기는 다음 기동부터, 프로젝트 폴더를 바꾸면 저장 감시를 바로 옮긴다.
// 소스 인코딩 · 매개변수 이름은 톰캣을 끈 뒤에만 바꾼다 (다음 컴파일이 전체를 다시 만든다)
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
    let java = edit.java.trim().trim_matches('"').replace('\\', "/");
    if !java.is_empty() && !Path::new(&java).is_file() {
        return Err(format!("java 가 없습니다: {java} (비우면 설정 › 경로의 java 를 씁니다)"));
    }
    let context_path = clean_context_path(&edit.context_path)?;
    if edit.max_parameter_count != -1 && edit.max_parameter_count < 1 {
        return Err("파라미터 수 상한은 1 이상이거나 -1(무제한)이어야 합니다.".into());
    }
    // 자바 문자 집합 이름에 쓸 수 있는 글자만 — 탭이 들어가면 상주 컴파일러 요청 줄이 깨진다
    let encoding = eclipse::clean_encoding(&edit.encoding);
    if !encoding.chars().all(|c| c.is_ascii_alphanumeric() || "-_.:+".contains(c)) {
        return Err(format!("소스 인코딩 이름이 올바르지 않습니다: {encoding} (예: UTF-8 · MS949 · EUC-KR)"));
    }
    // 설정 잠금을 잡기 전에 읽는다 — 두 잠금을 겹쳐 잡지 않는다 (import_servers 와 같은 순서)
    let stopped = rt(&app, srv).inner.lock().unwrap().status == Status::Stopped;
    let c = {
        let mut c = shared(&app).cfg.lock().unwrap();
        // 켜진 앱에는 옛 인코딩 · 옵션으로 만든 클래스가 올라가 있다. 바로 다시 컴파일하면 핫리로드가 클래스 수천 개를 한꺼번에 바꾸려 하고,
        // 미루면 옛 클래스와 새 클래스가 섞인다. 끈 뒤에 바꾸면 다음 기동이 비우고 한 번에 다시 만든다. 일부만 저장하지 않도록 아무것도 바꾸기 전에 막는다.
        let old = &c.servers[srv];
        if !stopped && (old.encoding != encoding || old.parameters != edit.parameters) {
            return Err(format!(
                "{} 톰캣을 끈 뒤에 소스 인코딩 · 매개변수 이름을 바꾸세요. 켜진 앱에는 지금 설정으로 만든 클래스가 들어 있습니다. 두 칸을 그대로 두면 나머지는 저장됩니다.",
                old.name
            ));
        }
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
        sv.java = java;
        sv.context_path = context_path;
        sv.max_parameter_count = edit.max_parameter_count;
        sv.encoding = encoding;
        sv.parameters = edit.parameters;
        (c.clone(), moved)
    };
    save_config(&c.0).map_err(|e| format!("설정을 저장하지 못했습니다: {e}"))?;
    if c.1 {
        start_watchers(&app);
    }
    emit_state(&app);
    let sv = &c.0.servers[srv];
    if c.1 && !sv.project.is_empty() && jar_count(&sv.deps_dir) == 0 {
        let a = app.clone();
        thread::spawn(move || match auto_deps_blocking(&a, srv) {
            Ok(m) => { let _ = a.emit("toast", m); }
            Err(e) => { let _ = a.emit("error", e); }
        });
    }
    Ok(c.1)
}

// 서버를 더한다 — 실행 상태 · 컴파일 줄을 함께 늘려 바로 보인다. 저장 감시는 프로젝트 폴더를 정하면 걸린다.
#[tauri::command]
fn add_server(app: AppHandle) -> Result<(), String> {
    let mut c = shared(&app).cfg.lock().unwrap().clone();
    let n = c.servers.len();
    let id = (n + 1..).map(|k| format!("app{k}")).find(|id| c.servers.iter().all(|s| &s.id != id)).unwrap_or_default();
    let mut sv = new_server(&home_dir(), &id, &format!("앱 {}", n + 1), n as u16);
    // 기본 포트가 이미 쓰이면 비어 있는 자리까지 올린다 — 다른 서버의 설정뿐 아니라 지금 이 PC 에서 듣고 있는 포트(IDE 톰캣 등)도 건너뛴다.
    // 바로가기는 「/」라 포트를 따라가므로 다시 만들지 않는다.
    let used: HashSet<u16> = c.servers.iter().flat_map(|s| [s.http_port, s.ajp_port, s.shutdown_port, s.debug_port]).collect();
    let mut k = n as u16;
    while [sv.http_port, sv.ajp_port, sv.shutdown_port, sv.debug_port].iter().any(|&x| used.contains(&x) || port_busy(x)) {
        k += 1;
        let t = new_server(&home_dir(), &id, &sv.name, k);
        (sv.http_port, sv.ajp_port, sv.shutdown_port, sv.debug_port) = (t.http_port, t.ajp_port, t.shutdown_port, t.debug_port);
    }
    c.servers.push(sv);
    save_config(&c).map_err(|e| format!("설정을 저장하지 못했습니다: {e}"))?;
    grow_rts(&app, c.servers.len());
    *shared(&app).cfg.lock().unwrap() = c;
    emit_state(&app);
    Ok(())
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
    tauri::async_runtime::spawn_blocking(move || {
        let mut list = match dir {
            Some(d) => {
                let d = PathBuf::from(d.trim());
                if !eclipse::is_workspace(&d) {
                    return Err(format!("이클립스 작업 공간이 아닙니다 (.metadata 의 서버 설정이 없습니다): {}", p(&d)));
                }
                vec![eclipse::read_workspace(&d)]
            }
            None => eclipse::find_workspaces(&projects).iter().map(|w| eclipse::read_workspace(w)).filter(|w| !w.servers.is_empty()).collect(),
        };
        // 「시작하기」 목록에 그 서버가 쓰던 자바 판을 붙인다
        for s in list.iter_mut().flat_map(|w| w.servers.iter_mut()).filter(|s| !s.java.is_empty()) {
            s.java_version = java_label(&s.java);
        }
        Ok(list)
    })
    .await
    .map_err(|e| e.to_string())?
}

// 이클립스 서버를 실행기 서버로 — 같은 프로젝트를 쓰는 서버가 있으면 그것을, 프로젝트가 빈 서버가 있으면 그것을 채우고, 없으면 더한다.
// 의존 jar 는 이클립스 배포본에서 옮긴다. 늘어난 서버 · 바뀐 프로젝트 폴더는 실행기를 다시 열지 않아도 바로 쓴다.
// 2026-10-29 : 서버 수가 바뀔 때만 실행기를 다시 열었다. 빈 서버를 채우거나 프로젝트를 바꾼 경우는 저장 감시가 옛 폴더에 남아 껐다 켜야 반영됐다.
// naming 은 사용자가 「JNDI 도 옮기기」를 켰을 때만 참이다 — DB 계정 · 비밀번호가 실행기 폴더로 복사되기 때문이다.
// 켤 때 저절로 가져올 때(auto_setup)는 묻지 못하므로 늘 거짓이고, 옮기지 않은 수와 다시 가져오는 길을 알린다.
fn import_servers(app: &AppHandle, picked: &[eclipse::EServer], naming: bool) -> Result<String, String> {
    if picked.is_empty() {
        return Err("가져올 서버가 없습니다 (프로젝트 폴더를 찾지 못했습니다).".into());
    }
    for i in 0..rt_count(app) {
        if rt(app, i).inner.lock().unwrap().status != Status::Stopped {
            return Err(format!("{} 톰캣을 끈 뒤에 가져오세요.", cfgs(app, i).1.name));
        }
    }
    let mut c = shared(app).cfg.lock().unwrap().clone();
    let mut jobs = Vec::new();
    let mut naming_jobs = Vec::new();
    let mut skipped = 0;
    let mut warned = 0;
    for es in picked {
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
        // 이클립스와 같은 주소 · 같은 요청 한도로 — 이클립스 server.xml 에 값이 없으면 톰캣 기본(루트 · 10000)으로 읽힌다
        sv.context_path = es.context_path.clone();
        sv.max_parameter_count = es.max_parameter_count;
        // 이클립스와 같은 인코딩 · -parameters 로 컴파일한다 — 바뀌었으면 다음 기동이 전체를 다시 만든다 (모든 서버가 꺼져 있다)
        sv.encoding = es.encoding.clone();
        sv.parameters = es.parameters;
        let mut compile_note = format!("{} 자바 컴파일 옵션: {}", sv.name, opts_label(sv));
        if es.encoding_unset {
            compile_note.push_str(" — 이클립스 설정에 소스 인코딩이 없어 UTF-8 로 둡니다. 한글이 깨지면 설정 › 서버 › 소스 인코딩을 MS949 로 바꾸세요");
        }
        log(app, None, "app", if es.encoding_unset { "warn" } else { "" }, compile_note);
        for a in &es.jvm_args {
            if !sv.jvm_args.contains(a) { sv.jvm_args.push(a.clone()); }
        }
        if !es.tomcat_home.is_empty() { c.tomcat_home = es.tomcat_home.clone(); }
        // 핫리로드를 끈 기동은 이클립스와 같은 JRE 로 — 포트처럼 찾은 것만 덮는다
        if !es.java.is_empty() {
            sv.java = es.java.clone();
            sv.eclipse_java = es.java.clone();
        }
        if !es.lib_dir.is_empty() { jobs.push((sv.clone(), es.lib_dir.clone())); }
        let movable = es.naming_full.iter().filter(|e| e.movable()).count();
        // 다시 가져오면 새로 읽은 것으로 덮는다 — {base} 는 이 서버의 톰캣 설정 폴더
        sv.eclipse_warnings = es.server_warnings.iter().map(|w| w.replace("{base}", &sv.catalina_base)).collect();
        if naming && movable > 0 {
            naming_jobs.push((sv.name.clone(), sv.catalina_base.clone(), es.naming_full.clone(), format!("이클립스 서버 「{}」", es.name)));
        } else {
            skipped += movable;
            if movable > 0 {
                sv.eclipse_warnings.push(format!("이클립스 서버에 DB 연결(JNDI) 설정 {movable}개가 있는데 옮기지 않았습니다 — 설정 › 서버 › 이클립스에서 가져오기에서 「JNDI 설정 {movable}개도 옮기기」를 켜고 다시 가져오세요"));
            }
        }
        warned += es.warnings.len();
    }
    save_config(&c).map_err(|e| format!("설정을 저장하지 못했습니다: {e}"))?;
    grow_rts(app, c.servers.len());
    *shared(app).cfg.lock().unwrap() = c;
    start_watchers(app);
    let mut notes = Vec::new();
    for (sv, lib) in jobs {
        let (name, deps) = (&sv.name, PathBuf::from(&sv.deps_dir));
        match eclipse::copy_deps(&lib, &sv.project, &deps) {
            Ok(n) => {
                let _ = fs::write(deps.join(STAMP_FILE), now_ms().to_string());
                log(app, None, "app", "ok", format!("{name} 의존 jar {n}개를 이클립스 배포본에서 옮겼습니다."));
                notes.push(format!("{name} jar {n}개"));
                redo_dedup(app, None, &sv);
            }
            Err(e) => {
                log(app, None, "app", "err", format!("{name} {e}"));
                notes.push(format!("{name} jar 못 옮김"));
            }
        }
    }
    for (name, base, entries, source) in naming_jobs {
        match write_naming(&base, &entries, &source) {
            Ok(n) => {
                // 로그에는 이름만 적는다 — 속성 값에 비밀번호가 있다
                let names: Vec<&str> = entries.iter().filter(|e| e.movable()).map(|e| e.name.as_str()).collect();
                log(app, None, "app", "ok", format!("{name} JNDI 설정 {n}개({})를 {} 에 옮겼습니다.", names.join(", "), p(&naming_file(&base))));
                notes.push(format!("{name} JNDI {n}개"));
            }
            Err(e) => {
                log(app, None, "app", "err", format!("{name} {e}"));
                notes.push(format!("{name} JNDI 못 옮김"));
            }
        }
    }
    if skipped > 0 {
        notes.push(format!("JNDI 설정 {skipped}개는 옮기지 않았습니다 (설정 › 서버 › 이클립스에서 가져오기)"));
    }
    if warned > 0 {
        notes.push(format!("이클립스와 다른 점 {warned}건 (상태 탭)"));
    }
    stop_daemon(app, "");
    let msg = format!("이클립스에서 서버 {}개를 가져왔습니다 · {}", picked.len(), notes.join(" · "));
    log(app, None, "app", "ok", msg.clone());
    emit_state(app);
    Ok(msg)
}

#[tauri::command]
async fn eclipse_import(app: AppHandle, ws: String, ids: Vec<String>, naming: bool) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || {
        // JNDI 속성은 화면을 거치지 않고 여기서 이클립스 파일을 다시 읽어 쓴다
        let picked: Vec<_> = eclipse::read_workspace(Path::new(&ws)).servers.into_iter().filter(|s| ids.contains(&s.id) && !s.project.is_empty()).collect();
        import_servers(&app, &picked, naming)
    })
    .await
    .map_err(|e| e.to_string())?
}

fn jar_count(dir: &str) -> usize {
    fs::read_dir(dir).map(|r| r.filter_map(Result::ok).filter(|e| e.path().extension().map_or(false, |x| x == "jar")).count()).unwrap_or(0)
}

// 의존 jar 를 알아서 채운다 — 이 프로젝트를 배포한 이클립스 서버가 이 PC 에 있으면 그 WEB-INF/lib 에서 옮기고(빠르고 사내 저장소가 필요 없다),
// 없으면 Maven 으로 받는다(따로 돌고 끝나면 알린다). 화면에 띄울 한 줄을 돌려준다.
fn auto_deps_blocking(app: &AppHandle, i: usize) -> Result<String, String> {
    let sv = cfgs(app, i).1;
    if sv.project.trim().is_empty() {
        return Err(format!("{} 프로젝트 폴더를 먼저 정하세요.", sv.name));
    }
    if rt(app, i).inner.lock().unwrap().status != Status::Stopped {
        return Err(format!("{} 톰캣을 끈 뒤에 채우세요. 실행 중에는 jar 가 잠겨 바꿀 수 없습니다.", sv.name));
    }
    let parent: Vec<PathBuf> = Path::new(&sv.project).parent().map(Path::to_path_buf).into_iter().collect();
    let found = eclipse::find_workspaces(&parent)
        .iter()
        .flat_map(|w| eclipse::read_workspace(w).servers)
        .find(|s| s.project.trim_end_matches('/').eq_ignore_ascii_case(sv.project.trim_end_matches('/')) && s.jars > 0);
    if let Some(es) = found {
        let n = eclipse::copy_deps(&es.lib_dir, &es.project, Path::new(&sv.deps_dir))?;
        let _ = fs::write(Path::new(&sv.deps_dir).join(STAMP_FILE), now_ms().to_string());
        stop_daemon(app, "");
        let msg = format!("{} 의존 jar {n}개를 이클립스 배포본에서 옮겼습니다.", sv.name);
        log(app, Some(i), "deps", "ok", msg.clone());
        redo_dedup(app, Some(i), &sv);
        emit_state(app);
        return Ok(msg);
    }
    if Path::new(&sv.project).join("pom.xml").is_file() {
        refresh_deps(app.clone(), i)?;
        return Ok(format!("{} 의존 jar 를 Maven 으로 받는 중입니다 (빌드 탭). 끝나면 알려 드립니다.", sv.name));
    }
    Err(format!("{} 의존 jar 를 채울 곳이 없습니다 — 이클립스 배포본도 pom.xml 도 없습니다. src/main/webapp/WEB-INF/lib 의 jar 만 씁니다.", sv.name))
}

#[tauri::command]
async fn auto_deps(app: AppHandle, srv: usize) -> Result<String, String> {
    check_srv(&app, srv)?;
    tauri::async_runtime::spawn_blocking(move || auto_deps_blocking(&app, srv)).await.map_err(|e| e.to_string())?
}

// 실행기를 켤 때 — 손댈 것 없이 쓸 수 있게 한다.
//   프로젝트를 정한 서버가 하나도 없으면 이클립스 작업 공간을 찾아 거기 있는 서버를 모두 가져온다. 못 찾으면 화면에 「시작하기」를 띄운다.
//   프로젝트는 있는데 의존 jar 가 하나도 없는 서버는 알아서 채운다.
fn auto_setup(app: AppHandle) {
    let c = shared(&app).cfg.lock().unwrap().clone();
    if c.servers.iter().all(|s| s.project.trim().is_empty()) {
        let found: Vec<eclipse::EServer> = eclipse::find_workspaces(&[])
            .iter()
            .map(|w| eclipse::read_workspace(w))
            .find(|w| w.servers.iter().any(|s| !s.project.is_empty()))
            .map(|w| w.servers.into_iter().filter(|s| !s.project.is_empty()).collect())
            .unwrap_or_default();
        if found.is_empty() {
            let _ = app.emit("setup-needed", ());
            return;
        }
        match import_servers(&app, &found, false) {
            Ok(msg) => {
                let _ = app.emit("toast", format!("{msg} · 시작을 누르면 됩니다"));
            }
            Err(e) => {
                log(&app, None, "app", "err", e);
                let _ = app.emit("setup-needed", ());
            }
        }
        return;
    }
    for (i, sv) in c.servers.iter().enumerate() {
        if !sv.project.trim().is_empty() && jar_count(&sv.deps_dir) == 0 {
            match auto_deps_blocking(&app, i) {
                Ok(msg) => { let _ = app.emit("toast", msg); }
                Err(e) => log(&app, Some(i), "deps", "warn", e),
            }
        }
    }
}

// 「시작하기 › 프로젝트 폴더 고르기」 — 그 서버의 프로젝트를 정하고 의존 jar 를 알아서 채운다
#[tauri::command]
async fn quick_project(app: AppHandle, srv: usize, project: String) -> Result<String, String> {
    check_srv(&app, srv)?;
    let project = project.trim().replace('\\', "/").trim_end_matches('/').to_string();
    if !Path::new(&project).join("src/main/webapp").is_dir() {
        return Err(format!("src/main/webapp 이 있는 프로젝트 폴더를 고르세요: {project}"));
    }
    {
        let mut c = shared(&app).cfg.lock().unwrap();
        let sv = &mut c.servers[srv];
        sv.project = project.clone();
        // 기본 이름(「앱」 · 「앱 2」)이면 폴더 이름으로
        if sv.name.starts_with("앱") {
            sv.name = Path::new(&project).file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
        }
        save_config(&c).map_err(|e| format!("설정을 저장하지 못했습니다: {e}"))?;
    }
    start_watchers(&app);
    emit_state(&app);
    let a = app.clone();
    let r = tauri::async_runtime::spawn_blocking(move || auto_deps_blocking(&a, srv)).await.map_err(|e| e.to_string())?;
    Ok(match r {
        Ok(m) => format!("프로젝트를 정했습니다 · {m}"),
        Err(e) => format!("프로젝트를 정했습니다 · {e}"),
    })
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

/* ---------- 버전 관리 쓰기 · Blame · 충돌 (2026.10.29~) ---------- */
//
// 쓰기는 svncli.rs(svn.exe) · gitwrite.rs(git) 에만 있다. 여기서는 막을 것을 먼저 막는다.
//   Allow write 가 꺼졌거나 · 그 서버의 버전 관리가 아니거나 · svn.exe 판이 맞지 않거나 · 같은 서버에서 다른 쓰기가 도는 중이면 거절
//   Conflicted 가 남은 채 Commit · 내가 고친 파일과 겹치는 리비전 되돌리기는 거절
// 끝나면(실패해도) 표지를 맞추고 vcs-dirty 를 한 번 보내 화면이 다시 읽게 한다.
// Blame · 충돌 세 쪽 보기는 읽기라 Allow write 와 상관없다.

struct VcsBusy {
    app: AppHandle,
    srv: usize,
    root: PathBuf,
}

impl Drop for VcsBusy {
    fn drop(&mut self) {
        let r = rt(&self.app, self.srv);
        // 쓰는 동안 바뀐 wc.db · .git 표지를 지금 값으로 맞춘다 — 그러지 않으면 감시가 같은 변경을 한 번 더 알린다
        if let Some(s) = svn::db_stamp(&self.root) {
            r.svn_stamp.store(s, Ordering::Relaxed);
        }
        if let Some(s) = git::stamp(&self.root) {
            r.git_stamp.store(s, Ordering::Relaxed);
        }
        r.vcs_busy.store(false, Ordering::SeqCst);
        let _ = self.app.emit("vcs-dirty", self.srv);
    }
}

fn vcs_begin(app: &AppHandle, srv: usize, kind: &str) -> Result<(ServerCfg, PathBuf, VcsBusy), String> {
    check_srv(app, srv)?;
    let s = cfgs(app, srv).1;
    if !s.vcs.write {
        return Err(format!("{} 은 「설정 › 버전 관리 › Allow write」가 꺼져 있습니다.", s.name));
    }
    if !vcs_kinds(&s).contains(&kind) {
        return Err(format!("{} 에서 쓰는 버전 관리가 아닙니다: {kind}", s.name));
    }
    if kind == "svn" {
        if let Some(why) = svn_block(&s) {
            return Err(why);
        }
    }
    let r = rt(app, srv);
    if r.vcs_busy.compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst).is_err() {
        return Err("이 서버에서 다른 버전 관리 작업이 도는 중입니다.".into());
    }
    let root = PathBuf::from(vcs_root(&s));
    Ok((s, root.clone(), VcsBusy { app: app.clone(), srv, root }))
}

fn svn_cli(s: &ServerCfg, wc: &Path) -> Result<svncli::Svn, String> {
    let p = svncli::probe(&s.vcs.svn_exe)?;
    Ok(svncli::Svn { exe: PathBuf::from(p.exe), wc: wc.to_path_buf() })
}

// 실행기에 넣어 둔 SVN 계정 — 없으면 svn 자신의 인증 캐시에만 기댄다
fn svn_cred(app: &AppHandle, wc: &Path) -> Option<(String, String)> {
    svn::wc_info(wc).ok().and_then(|i| creds_for(app, &i.url).ok())
}

fn vcs_kind_ok(kind: &str) -> Result<(), String> {
    if kind == "svn" || kind == "git" { Ok(()) } else { Err(format!("버전 관리 종류가 올바르지 않습니다: {kind}")) }
}

// 성공도 실패도 로그에 한 줄 — 무엇을 언제 저장소에 썼는지 나중에 찾을 수 있게
fn vcs_record<T>(app: &AppHandle, srv: usize, what: &str, r: &Result<T, String>, ok: impl Fn(&T) -> String) {
    match r {
        Ok(v) => log(app, Some(srv), "app", "", format!("{what} · {}", ok(v))),
        Err(e) => log(app, Some(srv), "app", "warn", format!("{what} 실패 · {}", e.trim_start_matches(NEED_LOGIN))),
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct VcsDone {
    msg: String,
    conflicts: Vec<String>, // 끝난 뒤 남은 Conflicted — 화면이 맨 위에 모아 보인다
}

fn conflicts_now(kind: &str, root: &Path) -> Vec<String> {
    let items = if kind == "git" { git::status(root).map(|s| s.items) } else { svn::status(root).map(|s| s.items) };
    items.map(|v| v.into_iter().filter(|i| i.conflict).map(|i| i.path).collect()).unwrap_or_default()
}

#[tauri::command]
async fn vcs_commit(app: AppHandle, srv: usize, kind: String, paths: Vec<String>, msg: String, allow_rule_break: Option<bool>) -> Result<String, String> {
    vcs_kind_ok(&kind)?;
    let msg = msg.trim().to_string();
    if msg.is_empty() {
        return Err("Commit message 를 먼저 적으세요.".into());
    }
    // 규칙은 경고만 한다 — 다만 확인 창에서 「그래도 Commit」을 고른 호출만 넘긴다
    check_srv(&app, srv)?;
    let broken = commitmsg::check(&cfgs(&app, srv).1.vcs.rule(), &msg);
    if !broken.is_empty() && allow_rule_break != Some(true) {
        return Err(format!("Commit message 가 규칙에 맞지 않습니다: {}", broken.join(" · ")));
    }
    if paths.is_empty() {
        return Err("고른 파일이 없습니다.".into());
    }
    for p in &paths {
        safe_rel(p)?;
    }
    let (s, root, busy) = vcs_begin(&app, srv, &kind)?;
    let cred = if kind == "svn" { svn_cred(&app, &root) } else { None };
    let r = tauri::async_runtime::spawn_blocking(move || -> Result<String, String> {
        let _busy = busy;
        if kind == "git" {
            let st = git::status(&root)?;
            let mut all = paths.clone();
            for p in &paths {
                let it = st.items.iter().find(|i| &i.path == p).ok_or_else(|| format!("Changes 목록에 없습니다: {p}"))?;
                if it.conflict {
                    return Err(format!("Conflicted 가 남아 있습니다: {p}. 먼저 Resolve 하세요."));
                }
                all.extend(it.orig.clone());
            }
            let id = gitwrite::commit(Path::new(&st.root), &msg, &all)?;
            return Ok(format!("Commit 했습니다 · {id} · 파일 {}개", paths.len()));
        }
        let st = svn::status(&root)?;
        let cli = svn_cli(&s, &root)?;
        let (mut adds, mut gone, mut new_dirs) = (Vec::new(), Vec::new(), Vec::new());
        for p in &paths {
            let it = st.items.iter().find(|i| &i.path == p).ok_or_else(|| format!("Changes 목록에 없습니다: {p}"))?;
            if it.conflict {
                return Err(format!("Conflicted 가 남아 있습니다: {p}. 먼저 Resolve 하세요."));
            }
            match it.code {
                "?" => {
                    adds.push(p.clone());
                    if it.dir {
                        new_dirs.push(p.clone());
                    }
                }
                "!" => gone.push(p.clone()),
                "A" if it.dir => new_dirs.push(p.clone()),
                _ => {}
            }
        }
        if !adds.is_empty() {
            cli.call(&svncli::add_args(&adds), cred.as_ref()).map_err(web_err)?;
        }
        if !gone.is_empty() {
            cli.call(&svncli::delete_args(&gone), cred.as_ref()).map_err(web_err)?;
        }
        // Commit 은 --depth empty 라 새 폴더의 안쪽을 하나하나 넣는다 (무시 목록에 걸린 것은 svn add 가 이미 뺐다)
        let mut targets = paths.clone();
        if !new_dirs.is_empty() {
            for it in svn::status(&root)?.items {
                if it.code == "A" && new_dirs.iter().any(|d| it.path.starts_with(&format!("{d}/"))) && !targets.contains(&it.path) {
                    targets.push(it.path);
                }
            }
        }
        let rev = cli.commit(&msg, &targets, cred.as_ref()).map_err(web_err)?;
        Ok(format!("Commit 했습니다 · {} · 파일 {}개", rev.map_or("리비전 모름".into(), |r| format!("r{r}")), targets.len()))
    })
    .await
    .map_err(|e| e.to_string())?;
    vcs_record(&app, srv, "Commit", &r, |m| m.clone());
    r
}

#[tauri::command]
async fn vcs_update(app: AppHandle, srv: usize, kind: String) -> Result<VcsDone, String> {
    vcs_kind_ok(&kind)?;
    let (s, root, busy) = vcs_begin(&app, srv, &kind)?;
    let cred = if kind == "svn" { svn_cred(&app, &root) } else { None };
    let what = if kind == "git" { "Pull" } else { "Update" };
    let r = tauri::async_runtime::spawn_blocking(move || -> Result<VcsDone, String> {
        let _busy = busy;
        if kind == "git" {
            let groot = git::toplevel(&root)?;
            let cmd = if s.vcs.git_update.trim().is_empty() { "git pull --ff-only" } else { s.vcs.git_update.as_str() };
            let last = gitwrite::pull(&groot, cmd)?;
            return Ok(VcsDone { msg: format!("Pull 했습니다{}", if last.is_empty() { String::new() } else { format!(" · {last}") }), conflicts: conflicts_now("git", &groot) });
        }
        svn_cli(&s, &root)?.call(&svncli::update_args(), cred.as_ref()).map_err(web_err)?;
        let rev = svn::wc_info(&root).map(|i| i.rev_max).unwrap_or(0);
        Ok(VcsDone { msg: format!("Update 했습니다 · r{rev}"), conflicts: conflicts_now("svn", &root) })
    })
    .await
    .map_err(|e| e.to_string())?;
    vcs_record(&app, srv, what, &r,|d| format!("{}{}", d.msg, if d.conflicts.is_empty() { String::new() } else { format!(" · Conflicted {}", d.conflicts.len()) }));
    r
}

#[tauri::command]
async fn vcs_revert(app: AppHandle, srv: usize, kind: String, paths: Vec<String>) -> Result<String, String> {
    vcs_kind_ok(&kind)?;
    if paths.is_empty() {
        return Err("고른 파일이 없습니다.".into());
    }
    for p in &paths {
        safe_rel(p)?;
    }
    let (s, root, busy) = vcs_begin(&app, srv, &kind)?;
    let cred = if kind == "svn" { svn_cred(&app, &root) } else { None };
    let r = tauri::async_runtime::spawn_blocking(move || -> Result<String, String> {
        let _busy = busy;
        if kind == "git" {
            let st = git::status(&root)?;
            let items: Vec<gitwrite::RevertItem> = paths
                .iter()
                .filter_map(|p| st.items.iter().find(|i| &i.path == p))
                .map(|i| gitwrite::RevertItem { path: i.path.clone(), code: i.code.to_string(), orig: i.orig.clone() })
                .collect();
            let n = items.iter().filter(|i| i.code != "?").count();
            gitwrite::revert(Path::new(&st.root), &items)?;
            return Ok(format!("Revert 했습니다 · 파일 {n}개"));
        }
        let st = svn::status(&root)?;
        // 새로 더한 · 지운 · 없어진 폴더는 안쪽까지, 나머지는 그 경로 하나만 — 「.」 의 속성만 되돌리려다 전체가 되돌아가지 않게
        let (mut deep, mut shallow) = (Vec::new(), Vec::new());
        for p in &paths {
            let Some(it) = st.items.iter().find(|i| &i.path == p) else { continue };
            if it.code == "?" {
                continue; // 버전 관리 밖 파일은 되돌릴 것이 없다 (지우지 않는다)
            }
            if it.dir && ["A", "D", "!", "~"].contains(&it.code) { deep.push(p.clone()) } else { shallow.push(p.clone()) }
        }
        let cli = svn_cli(&s, &root)?;
        for (d, v) in [(false, &shallow), (true, &deep)] {
            if !v.is_empty() {
                cli.call(&svncli::revert_args(d, v), cred.as_ref()).map_err(web_err)?;
            }
        }
        Ok(format!("Revert 했습니다 · 파일 {}개", shallow.len() + deep.len()))
    })
    .await
    .map_err(|e| e.to_string())?;
    vcs_record(&app, srv, "Revert", &r, |m| m.clone());
    r
}

#[tauri::command]
async fn vcs_resolve(app: AppHandle, srv: usize, kind: String, path: String, accept: String) -> Result<String, String> {
    vcs_kind_ok(&kind)?;
    safe_rel(&path)?;
    let (s, root, busy) = vcs_begin(&app, srv, &kind)?;
    let cred = if kind == "svn" { svn_cred(&app, &root) } else { None };
    let r = tauri::async_runtime::spawn_blocking(move || -> Result<String, String> {
        let _busy = busy;
        let ci = if kind == "git" { git::conflict_info(&root, &path)? } else { svn::conflict_info(&root, &path)? };
        if ci.kinds.contains(&"tree") && accept != "working" {
            return Err("tree conflict 은 Use mine · Use theirs 로 풀리지 않습니다. 파일을 맞춘 뒤 Mark as resolved 를 누르거나 TortoiseSVN 으로 푸세요.".into());
        }
        if kind == "git" {
            gitwrite::resolve(&git::toplevel(&root)?, &path, &accept)?;
        } else {
            svn_cli(&s, &root)?.call(&svncli::resolve_args(&accept, &path)?, cred.as_ref()).map_err(web_err)?;
        }
        let how = match accept.as_str() {
            "mine" => "Use mine",
            "theirs" => "Use theirs",
            _ => "Mark as resolved",
        };
        Ok(format!("Resolve 했습니다 · {path} · {how}"))
    })
    .await
    .map_err(|e| e.to_string())?;
    vcs_record(&app, srv, "Resolve", &r, |m| m.clone());
    r
}

// 리비전(커밋) 하나의 변경을 거꾸로 작업복사본에 적용한다. 결과는 Changes 에 남고 Commit 은 사용자가 따로 한다.
#[tauri::command]
async fn vcs_revert_revision(app: AppHandle, srv: usize, kind: String, rev: String) -> Result<VcsDone, String> {
    vcs_kind_ok(&kind)?;
    let (s, root, busy) = vcs_begin(&app, srv, &kind)?;
    let cred = if kind == "svn" { Some(svn::wc_info(&root).and_then(|i| creds_for(&app, &i.url))) } else { None };
    let r = tauri::async_runtime::spawn_blocking(move || -> Result<VcsDone, String> {
        let _busy = busy;
        // 내가 고친 파일과 겹치면 되돌린 변경과 내 변경이 한 파일에 섞여 가리지 못한다
        let check = |changed: Vec<String>, mine: Vec<String>| -> Result<(), String> {
            if changed.is_empty() {
                return Err("이 리비전은 작업복사본 안을 바꾸지 않았습니다.".into());
            }
            let both: Vec<String> = changed.into_iter().filter(|p| mine.contains(p)).collect();
            if both.is_empty() { Ok(()) } else { Err(format!("내가 고친 파일과 겹칩니다: {}. 먼저 Commit 하거나 Revert 하세요.", both.join(", "))) }
        };
        if kind == "git" {
            let st = git::status(&root)?;
            let groot = PathBuf::from(&st.root);
            let mine = st.items.into_iter().filter(|i| i.code != "?").map(|i| i.path).collect();
            check(gitwrite::changed_paths(&groot, &rev)?, mine)?;
            let r = gitwrite::revert_commit(&groot, &rev);
            let conflicts = conflicts_now("git", &groot);
            r?;
            let short: String = rev.chars().take(8).collect();
            return Ok(VcsDone { msg: format!("{short} 의 변경을 거꾸로 적용했습니다. 확인한 뒤 Commit 하세요."), conflicts });
        }
        let n: i64 = rev.trim_start_matches('r').parse().map_err(|_| format!("리비전이 올바르지 않습니다: {rev}"))?;
        let cred = cred.unwrap_or_else(|| Err("SVN 계정이 필요합니다.".into()))?;
        let info = svn::wc_info(&root)?;
        let entries = svnweb::log(&info.url, &info.repos_path, &cred, n, n, 1, "").map_err(web_err)?;
        let changed = entries.iter().flat_map(|e| e.paths.iter()).filter(|c| c.inside && !c.dir).map(|c| c.path.clone()).collect();
        let mine = svn::status(&root)?.items.into_iter().filter(|i| i.code != "?").map(|i| i.path).collect();
        check(changed, mine)?;
        svn_cli(&s, &root)?.call(&svncli::reverse_merge_args(n, &info.url), Some(&cred)).map_err(web_err)?;
        Ok(VcsDone { msg: format!("r{n} 의 변경을 거꾸로 적용했습니다. 확인한 뒤 Commit 하세요."), conflicts: conflicts_now("svn", &root) })
    })
    .await
    .map_err(|e| e.to_string())?;
    vcs_record(&app, srv, "Revert revision", &r, |d| d.msg.clone());
    r
}

// 줄마다 마지막으로 고친 리비전 · 작성자 (읽기 — Allow write 와 상관없다). SVN 은 BASE, Git 은 지금 파일 기준이다.
#[tauri::command]
async fn vcs_blame(app: AppHandle, srv: usize, kind: String, path: String) -> Result<svn::Blame, String> {
    vcs_kind_ok(&kind)?;
    safe_rel(&path)?;
    if path.is_empty() {
        return Err("파일 경로가 올바르지 않습니다.".into());
    }
    check_srv(&app, srv)?;
    let s = cfgs(&app, srv).1;
    let root = PathBuf::from(vcs_root(&s));
    let cred = if kind == "svn" { svn_cred(&app, &root) } else { None };
    tauri::async_runtime::spawn_blocking(move || -> Result<svn::Blame, String> {
        if kind == "git" {
            return git::blame(&root, &path);
        }
        let base = svn::base_content(&root, &path).ok_or("BASE 가 없습니다 (새로 더한 파일이거나 폴더입니다).")?;
        let note = |m: &str| Ok(svn::Blame { path: path.clone(), note: Some(m.into()), lines: Vec::new() });
        if base.len() > 3 * 1024 * 1024 {
            return note("파일이 커서(3MB 넘음) Blame 을 보이지 않습니다.");
        }
        if base.iter().take(8192).any(|&b| b == 0) {
            return note("바이너리 파일이라 Blame 을 보이지 않습니다.");
        }
        let xml = svn_cli(&s, &root)?.call(&svncli::blame_args(&path), cred.as_ref()).map_err(web_err)?;
        let lines = svncli::join_blame(&svn::decode(&base), svncli::parse_blame(&xml)?);
        Ok(svn::Blame { path, note: None, lines })
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
async fn vcs_conflict(app: AppHandle, srv: usize, kind: String, path: String) -> Result<svn::ConflictInfo, String> {
    vcs_kind_ok(&kind)?;
    safe_rel(&path)?;
    check_srv(&app, srv)?;
    let root = PathBuf::from(vcs_root(&cfgs(&app, srv).1));
    tauri::async_runtime::spawn_blocking(move || if kind == "git" { git::conflict_info(&root, &path) } else { svn::conflict_info(&root, &path) })
        .await
        .map_err(|e| e.to_string())?
}

// 충돌 세 쪽 — side : mine(BASE ↔ 내 것) · theirs(BASE ↔ 저장소 것)
#[tauri::command]
async fn vcs_conflict_diff(app: AppHandle, srv: usize, kind: String, path: String, side: String) -> Result<svn::Diff, String> {
    vcs_kind_ok(&kind)?;
    safe_rel(&path)?;
    check_srv(&app, srv)?;
    let root = PathBuf::from(vcs_root(&cfgs(&app, srv).1));
    tauri::async_runtime::spawn_blocking(move || if kind == "git" { git::conflict_diff(&root, &path, &side) } else { svn::conflict_diff(&root, &path, &side) })
        .await
        .map_err(|e| e.to_string())?
}

/* --- Git Fetch · Push --- */

// Fetch 는 작업 파일을 바꾸지 않지만 .git 의 원격 추적 브랜치를 고치므로 다른 쓰기처럼 Allow write 를 본다
#[tauri::command]
async fn vcs_fetch(app: AppHandle, srv: usize) -> Result<String, String> {
    let (_, root, busy) = vcs_begin(&app, srv, "git")?;
    let r = tauri::async_runtime::spawn_blocking(move || -> Result<String, String> {
        let _busy = busy;
        gitwrite::fetch(&git::toplevel(&root)?)?;
        let st = git::status(&root)?;
        Ok(match &st.upstream {
            Some(u) => format!("Fetch 했습니다 · {u} · 받을 커밋 {} · 올릴 커밋 {}", st.behind, st.ahead),
            None => "Fetch 했습니다 · 지금 브랜치에는 원격 추적 브랜치가 없습니다".into(),
        })
    })
    .await
    .map_err(|e| e.to_string())?;
    vcs_record(&app, srv, "Fetch", &r, |m| m.clone());
    r
}

// Push 확인 창이 무엇이 올라가는지 보이려고 부른다 (읽기)
#[tauri::command]
async fn git_outgoing(app: AppHandle, srv: usize) -> Result<git::Outgoing, String> {
    let root = git_root(&app, srv)?;
    tauri::async_runtime::spawn_blocking(move || git::outgoing(&root)).await.map_err(|e| e.to_string())?
}

// 바깥 저장소에 남아 되돌리기 어렵다 — 화면은 늘 올라갈 커밋 목록을 확인 창에 보인 뒤에만 부른다
#[tauri::command]
async fn vcs_push(app: AppHandle, srv: usize) -> Result<String, String> {
    let (_, root, busy) = vcs_begin(&app, srv, "git")?;
    let r = tauri::async_runtime::spawn_blocking(move || -> Result<String, String> {
        let _busy = busy;
        let groot = git::toplevel(&root)?;
        gitwrite::push_target(&groot)?;
        let n = git::outgoing(&groot)?.entries.len();
        if n == 0 {
            return Err("올릴 커밋이 없습니다.".into());
        }
        let to = gitwrite::push(&groot)?;
        Ok(format!("Push 했습니다 · {to} · 커밋 {n}개"))
    })
    .await
    .map_err(|e| e.to_string())?;
    vcs_record(&app, srv, "Push", &r, |m| m.clone());
    r
}

/* --- Commit message 자동 생성 · 규칙 검사 (commitmsg.rs) --- */

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct MsgGen {
    msg: String,
    problems: Vec<String>, // 만든 메시지가 규칙에 어긋난 까닭 — 칸 아래에 보인다
}

// 저장소에 쓰지 않으므로 Allow write 와 상관없다. 대신 「Commit message 생성 명령」이 비면 하지 않는다 (소스가 밖으로 나가는 일)
#[tauri::command]
async fn gen_commit_msg(app: AppHandle, srv: usize, kind: String, paths: Vec<String>) -> Result<MsgGen, String> {
    vcs_kind_ok(&kind)?;
    if paths.is_empty() {
        return Err("고른 파일이 없습니다.".into());
    }
    for x in &paths {
        safe_rel(x)?;
    }
    check_srv(&app, srv)?;
    let (c, s) = cfgs(&app, srv);
    let cmd = c.msg_cmd.trim().to_string();
    if cmd.is_empty() {
        return Err("「설정 › 경로 › Commit message 생성 명령」을 먼저 적으세요.".into());
    }
    let root = PathBuf::from(vcs_root(&s));
    let rule = s.vcs.rule();
    let (n, t0) = (paths.len(), Instant::now());
    let r = tauri::async_runtime::spawn_blocking(move || -> Result<MsgGen, String> {
        // 이슈 번호는 Git 이면 브랜치, SVN 이면 저장소 경로(branches/FDR-112 꼴)에서 찾는다
        let (items, place) = if kind == "git" {
            let st = git::status(&root)?;
            let place = if st.branch.is_empty() { format!("분리된 HEAD {}", st.head) } else { st.branch.clone() };
            (st.items, place)
        } else {
            let st = svn::status(&root)?;
            (st.items, svn::wc_info(&root).map(|i| i.url).unwrap_or_else(|_| p(&root)))
        };
        let files: Vec<commitmsg::FileDiff> = paths
            .iter()
            .map(|path| {
                let it = items.iter().find(|i| &i.path == path);
                let d = if kind == "git" { git::diff(&root, path, it.and_then(|i| i.orig.as_deref())) } else { svn::diff(&root, path) };
                commitmsg::FileDiff { path: path.clone(), label: it.map_or("Modified", |i| i.label).to_string(), diff: d.ok().and_then(|d| commitmsg::diff_text(&d)) }
            })
            .collect();
        let issue = commitmsg::issue_of(&rule.issue_pattern, &place).or_else(|| commitmsg::issue_of(&rule.issue_pattern, &p(&root)));
        let msg = commitmsg::run(&cmd, &commitmsg::build_prompt(&rule, issue.as_deref(), &place, &files))?;
        Ok(MsgGen { problems: commitmsg::check(&rule, &msg), msg })
    })
    .await
    .map_err(|e| e.to_string())?;
    // 무엇을 넘겼는지(프롬프트 · Diff)는 남기지 않는다 — 언제 몇 개를 보냈는지만
    match &r {
        Ok(_) => log(&app, Some(srv), "app", "", format!("Commit message 를 만들었습니다 · 파일 {n}개 · {:.1}초", t0.elapsed().as_secs_f32())),
        Err(e) => log(&app, Some(srv), "app", "warn", format!("Commit message 생성 실패 · {e}")),
    }
    r
}

// 메시지 칸을 고칠 때마다 부른다 — 화면과 vcs_commit 이 같은 규칙 엔진을 쓰게 하려는 것
#[tauri::command]
fn check_commit_msg(app: AppHandle, srv: usize, msg: String) -> Result<Vec<String>, String> {
    check_srv(&app, srv)?;
    Ok(commitmsg::check(&cfgs(&app, srv).1.vcs.rule(), &msg))
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
    let exe = vcs.svn_exe.trim();
    if !exe.is_empty() && !Path::new(exe).is_file() {
        return Err(format!("svn.exe 가 없습니다: {exe}"));
    }
    commitmsg::validate(&vcs.rule())?;
    let c = {
        let mut c = shared(&app).cfg.lock().unwrap();
        c.servers[srv].vcs = VcsCfg {
            kind: vcs.kind,
            root: root.replace('\\', "/"),
            commit_hint: vcs.commit_hint.trim().to_string(),
            svn_update: vcs.svn_update.trim().to_string(),
            git_update: vcs.git_update.trim().to_string(),
            write: vcs.write,
            svn_exe: vcs.svn_exe.trim().replace('\\', "/"),
            msg_rule: vcs.msg_rule.trim().to_string(),
            issue_pattern: vcs.issue_pattern.trim().to_string(),
            subject_pattern: vcs.subject_pattern.trim().to_string(),
            subject_max: vcs.subject_max,
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
    let shared = Shared {
        auto_compile: AtomicBool::new(cfg.auto_compile),
        cfg: Mutex::new(cfg),
        rts: RwLock::new(Vec::new()), // 화면이 뜨기 전 setup 에서 채운다 (컴파일 줄은 앱 핸들이 있어야 띄운다)
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
            let n = h.state::<Shared>().cfg.lock().unwrap().servers.len();
            grow_rts(&h, n);
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
            let h8 = h.clone();
            thread::spawn(move || {
                // 화면이 이벤트를 받을 수 있게 조금 기다린다
                thread::sleep(Duration::from_millis(1500));
                auto_setup(h8)
            });
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
            git_status, git_diff, git_incoming, git_log, git_rev_diff,
            vcs_commit, vcs_update, vcs_revert, vcs_resolve, vcs_revert_revision, vcs_blame, vcs_conflict, vcs_conflict_diff, set_vcs, set_paths, set_server, add_server, pick_project,
            vcs_fetch, vcs_push, git_outgoing, gen_commit_msg, check_commit_msg,
            update_check, update_install, changelog, eclipse_scan, eclipse_import, quick_project, pick_dir, auto_deps, dedup_deps, undo_dedup, apply_scan_hint
        ])
        .run(tauri::generate_context!())
        .expect("실행기를 띄우지 못했습니다");
}

#[cfg(test)]
mod tests {
    use super::*;

    const PROJ: &str = "D:/work/shop";

    #[test]
    fn 자바_판을_읽는다() {
        assert_eq!(parse_java_version("1.8.0_181"), Some((8, "8u181".into())));
        assert_eq!(parse_java_version("17.0.8"), Some((17, "17.0.8".into())));
        assert_eq!(parse_java_version("11"), Some((11, "11".into())));
        assert_eq!(parse_java_version(""), None);
        // release 가 없으면 폴더 이름으로 짐작 · 모르면 8 (지금까지와 같은 인자)
        assert_eq!(java_major("C:/없는/jdk-17.0.2/bin/java.exe"), 17);
        assert_eq!(java_major("C:/없는/Eclipse Adoptium/jdk-21.0.1.12-hotspot/bin/java.exe"), 21);
        assert_eq!(java_major("C:/없는/jdk1.8.0_301/jre/bin/java.exe"), 8);
        assert_eq!(java_major("C:/없는/app2024/bin/java.exe"), 8);
        let dir = std::env::temp_dir().join(format!("tomcatrun-java-{}", std::process::id()));
        fs::create_dir_all(dir.join("jre/bin")).unwrap();
        fs::write(dir.join("release"), "IMPLEMENTOR=\"x\"\nJAVA_VERSION=\"1.8.0_392\"\n").unwrap();
        // JDK 8 의 jre/bin/java.exe 는 두 단계 위의 release
        assert_eq!(java_version(&p(&dir.join("jre/bin/java.exe"))), Some((8, "8u392".into())));
        let _ = fs::remove_dir_all(&dir);
    }

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
        // 2026.10.29 전에 쓴 파일에는 접속 경로 · 파라미터 수 상한이 없다 — 루트 · 10000 (0 이면 톰캣이 파라미터를 하나도 받지 않는다)
        assert_eq!((c.servers[0].context_path.as_str(), c.servers[0].max_parameter_count), ("", 10000));
        assert!(!c.lazy_init && !c.no_schedule && !c.mapper_hotswap && !c.skip_scan);
        // 기본 설정은 빈 서버 하나 — 프로젝트 · 접속 주소를 코드에 두지 않는다
        let def = Config::default();
        assert_eq!(def.servers.len(), 1);
        assert!(def.servers[0].project.is_empty());
        // 바로가기는 「/」 하나 — 화면이 그 서버의 HTTP 포트를 붙여 연다
        assert_eq!(def.servers[0].links[0].url, "/");
        // 2026-10-28 에 지운 「SVN 로그인 안내」(vcs.accountNote)가 남은 파일도 읽히고, 다시 쓰면 빠진다
        let c = parse_config(r#"{"servers":[{"vcs":{"kind":"svn","accountNote":"x"}}]}"#).unwrap();
        assert_eq!(c.servers[0].vcs.kind, "svn");
        assert!(!serde_json::to_string(&c).unwrap().contains("accountNote"));
        // 2026-10-29 전에 쓴 파일에는 write · svnExe 가 없다 — 읽으면 쓰기는 꺼져 있다
        assert!(!c.servers[0].vcs.write && c.servers[0].vcs.svn_exe.is_empty());
        assert!(!VcsCfg::default().write);
        // 메시지 규칙 칸이 없던 파일도 읽힌다 — 규칙은 비어 있고(검사 안 함) 생성 명령도 비어 있다(아무것도 밖으로 안 나간다)
        let v = &c.servers[0].vcs;
        assert!(v.subject_pattern.is_empty() && v.subject_max == 0 && !v.issue_pattern.is_empty());
        assert!(c.msg_cmd.is_empty() && def.msg_cmd.is_empty());
        assert!(commitmsg::check(&v.rule(), "아무 말").is_empty());
    }

    #[test]
    fn 컴파일_옵션이_바뀌면_알아챈다() {
        let dir = std::env::temp_dir().join(format!("tomcatrun-opts-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let mut sv = new_server(&dir, "a", "앱", 0);
        // 옛 설정 파일에는 두 칸이 없다 — UTF-8 · 끔
        let old = parse_config(r#"{"servers":[{"id":"a"}]}"#).unwrap();
        assert_eq!((old.servers[0].encoding.as_str(), old.servers[0].parameters), ("", false));
        // 옵션 파일이 없는 옛 컴파일 결과는 그때 쓰던 UTF-8 · 끔과 같다고 본다 — 업데이트만으로 전체 컴파일을 하지 않는다
        assert_eq!(read_compile_opts(&sv.classes_dir), compile_opts(&sv));
        sv.encoding = "MS949".into();
        sv.parameters = true;
        assert_ne!(read_compile_opts(&sv.classes_dir), compile_opts(&sv));
        fs::create_dir_all(&sv.classes_dir).unwrap();
        fs::write(Path::new(&sv.classes_dir).join(COMPILE_OPTS_FILE), compile_opts(&sv)).unwrap();
        assert_eq!(read_compile_opts(&sv.classes_dir), compile_opts(&sv));
        // 상주 컴파일러 요청 — 옛 네 칸 뒤에 인코딩 · -parameters
        let req = compile_request("C:/c", "C:/c;D:/d/*", Path::new("C:/t/args.txt"), &sv);
        assert_eq!(req, "COMPILE\tC:/c\tC:/c;D:/d/*\tC:/t/args.txt\tMS949\ttrue");
        let _ = fs::remove_dir_all(&dir);
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
        assert!(props.contains(&"-Dtomcatrun.contextPath=".to_string()) && props.contains(&"-Dtomcatrun.maxParameterCount=10000".to_string()));
        sv.context_path = "/shop".into();
        sv.max_parameter_count = 0;
        let props = server_props(&cfg, &sv);
        assert!(props.contains(&"-Dtomcatrun.contextPath=/shop".to_string()) && props.contains(&"-Dtomcatrun.maxParameterCount=10000".to_string()));
    }

    #[test]
    fn 접속_경로는_톰캣_표기로_받는다() {
        for (raw, want) in [("", ""), ("/", ""), (" //", ""), ("shop", "/shop"), ("/shop/", "/shop"), ("/a/b", "/a/b")] {
            assert_eq!(clean_context_path(raw).unwrap(), want, "{raw}");
        }
        for bad in ["/a b", "/a?x", "/a#", r"\a", "/a//b", "/../x"] {
            assert!(clean_context_path(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn 옛_server_xml_은_손대지_않은_복사본만_새_원본으로_바꾼다() {
        let dir = std::env::temp_dir().join(format!("tomcatrun-upgrade-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let tpl = include_str!("../../base/conf/server.xml");
        // 옛 원본 — 박힌 값 두 곳 · 머리말 설명이 다르고 줄바꿈이 CRLF
        let old = tpl
            .replace(r#"maxParameterCount="${tomcatrun.maxParameterCount}""#, r#"maxParameterCount="1000""#)
            .replace(r#"<Context path="${tomcatrun.contextPath}""#, r#"<Context path="""#)
            .replace("tomcatrun 로컬 개발 톰캣", "옛 설명")
            .replace('\n', "\r\n");
        let f = dir.join("server.xml");
        fs::write(&f, &old).unwrap();
        assert_eq!(upgrade_server_xml(&f, tpl).unwrap(), None);
        assert_eq!(fs::read_to_string(&f).unwrap(), tpl);
        assert_eq!(fs::read_to_string(dir.join("server.xml.bak")).unwrap(), old);
        // 이미 새 원본이면 그대로
        assert_eq!(upgrade_server_xml(&f, tpl).unwrap(), None);
        // 사용자가 고친 파일은 건드리지 않고 경고만
        let mine = old.replace(r#"connectionTimeout="20000""#, r#"connectionTimeout="60000""#);
        fs::write(&f, &mine).unwrap();
        let w = upgrade_server_xml(&f, tpl).unwrap().unwrap();
        assert!(w.contains("접속 경로 · 파라미터 수 상한") && w.contains("${tomcatrun.contextPath}"), "{w}");
        assert_eq!(fs::read_to_string(&f).unwrap(), mine);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn jndi_는_서버별_파일에_쓰고_손으로_만든_것은_남긴다() {
        let base = std::env::temp_dir().join(format!("tomcatrun-naming-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&base);
        let b = p(&base);
        let e = |kind: &str, name: &str, pw: &str| eclipse::NamingEntry {
            kind: kind.into(),
            name: name.into(),
            attrs: vec![("name".into(), name.into()), ("password".into(), pw.into()), ("url".into(), "jdbc:x?a=1&b=2".into())],
            from: "server.xml".into(),
        };
        // 옮길 것이 없으면 파일을 만들지 않는다
        assert_eq!(write_naming(&b, &[e("ResourceLink", "jdbc/lost", "")], "t").unwrap(), 0);
        assert!(!naming_file(&b).exists());
        // 사용자가 만든 파일은 .bak 으로 남긴다
        fs::create_dir_all(naming_file(&b).parent().unwrap()).unwrap();
        fs::write(naming_file(&b), "<Context/>").unwrap();
        assert_eq!(write_naming(&b, &[e("Resource", "jdbc/a", "p\"<1"), e("ResourceLink", "jdbc/lost", "")], "이클립스 서버 「앱」").unwrap(), 1);
        let bak = base.join("conf/Catalina/localhost/context.xml.default.bak");
        assert_eq!(fs::read_to_string(&bak).unwrap(), "<Context/>");
        let xml = fs::read_to_string(naming_file(&b)).unwrap();
        assert!(xml.contains(r#"<Resource name="jdbc/a" password="p&quot;&lt;1" url="jdbc:x?a=1&amp;b=2"/>"#), "{xml}");
        assert!(!xml.contains("jdbc/lost"));
        assert!(roxmltree::Document::parse(&xml).is_ok());
        // 실행기가 쓴 파일은 .bak 없이 갈아 쓴다
        fs::remove_file(&bak).unwrap();
        write_naming(&b, &[e("Resource", "jdbc/b", "x")], "t").unwrap();
        assert!(!bak.exists() && fs::read_to_string(naming_file(&b)).unwrap().contains("jdbc/b"));
        let _ = fs::remove_dir_all(&base);
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
        assert_eq!(session_file("C:/t/base", "").as_path(), Path::new("C:/t/base/work/Catalina/localhost/ROOT/SESSIONS.ser"));
        assert_eq!(session_file("C:/t/base", "/shop").as_path(), Path::new("C:/t/base/work/Catalina/localhost/shop/SESSIONS.ser"));
        assert_eq!(session_file("C:/t/base", "/a/b").as_path(), Path::new("C:/t/base/work/Catalina/localhost/a#b/SESSIONS.ser"));
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
    fn spring_빈_다시_등록_줄에서_이름을_뽑는다() {
        // 실제 로그 그대로 (HotswapAgent 1.4.1)
        let line = "HOTSWAP AGENT: 17:56:29.875 RELOAD (org.hotswap.agent.plugin.spring.scanner.ClassPathBeanDefinitionScannerAgent) - Registering Spring bean 'fdrController'";
        assert_eq!(bean_name(line), Some("fdrController"));
        assert_eq!(bean_name("HOTSWAP AGENT: 17:56:29.894 INFO (x) - Spring: clearing HandlerMapping for class a.B"), None);
        assert_eq!(some_names(&["a".into()]), "a");
        assert_eq!(some_names(&["a".into(), "b".into(), "c".into()]), "a 외 2개");
        assert_eq!(some_names(&[]), "");
    }

    #[test]
    fn 클래스_교체_실패를_잡고_이름을_붙인다() {
        // 실제 로그 그대로 — 실패 줄에는 「HOTSWAP AGENT」 머리가 없다
        assert!(is_redefine_failure(r#"Exception in thread "Thread-352" java.lang.IllegalStateException: Unable to redefine classes"#));
        assert_eq!(classify("Caused by: java.lang.UnsupportedOperationException: class redefinition failed: attempted to change superclass or interfaces"), "err");
        assert!(!is_redefine_failure("HOTSWAP AGENT: 18:12:39.488 RELOAD (x) - Registering Spring bean 'a'"));
        let line = "HOTSWAP AGENT: 18:12:39.132 RELOAD (org.hotswap.agent.config.PluginManager) - Reloading classes [com.example.OrderServiceImpl$4, com.example.OrderServiceImpl$1, com.example.OrderServiceImpl$TxProxy, com.example.OrderServiceImpl, com.example.web.OrderController$Hub]";
        assert_eq!(reload_names(line).as_deref(), Some("OrderServiceImpl 외 1개"));
        assert_eq!(reload_names("Reloading classes [a.B]").as_deref(), Some("B"));
        assert!(reload_names("Registering directory").is_none());
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

    #[test]
    fn jstat_columns() {
        let gc = "  S0     S1     E      O      M     CCS    YGC     YGCT    FGC    FGCT     GCT   \n  0.00  99.12  41.20  63.05  97.11  93.40     41    1.234     3    0.766    2.000\n";
        let cls = "Loaded  Bytes  Unloaded  Bytes     Time   \n 41233 80412.3       12    20.1       6.12\n";
        let j = parse_jstat(gc, cls).expect("읽힌다");
        assert_eq!((j.gc_ms, j.gc_count, j.classes, j.class_ms), (2000, 44, 41233, 6120));
        assert!(parse_jstat("", cls).is_none());
    }

    #[test]
    fn startup_stacks_pick_webapp_thread() {
        let dump = "\"main\" #1 prio=5\n   java.lang.Thread.State: WAITING\n\tat java.util.concurrent.FutureTask.get(FutureTask.java:191)\n\tat org.apache.catalina.core.ContainerBase.startInternal(ContainerBase.java:942)\n\n\
                    \"localhost-startStop-1\" #20 daemon\n\tat java.util.zip.ZipFile.read(Native Method)\n\tat org.apache.tomcat.util.scan.StandardJarScanner.process(StandardJarScanner.java:350)\n\tat org.apache.catalina.startup.ContextConfig.webConfig(ContextConfig.java:1144)\n";
        let s = startup_stacks(dump);
        assert_eq!(s, vec![vec!["org.apache.tomcat.util.scan.StandardJarScanner.process".to_string(), "org.apache.catalina.startup.ContextConfig.webConfig".to_string()]]);
        // 기동 스레드가 아직 없으면 main
        let only_main = "\"main\" #1\n\tat org.apache.catalina.startup.Catalina.load(Catalina.java:600)\n";
        assert_eq!(startup_stacks(only_main)[0][0], "org.apache.catalina.startup.Catalina.load");
        let report = profile_report("앱", &[(1000, s.clone()), (3000, s)], &[(2, 2000), (4, 5000)]);
        assert!(report.contains("2회 (100%)  org.apache.tomcat.util.scan.StandardJarScanner.process"), "{report}");
        assert!(report.contains("웹앱 준비 (jar 스캔 등) · 스택 1개") && report.contains("Spring MVC · 마무리 · 스택 1개"), "{report}");
    }
}
