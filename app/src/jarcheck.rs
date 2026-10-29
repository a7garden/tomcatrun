// 의존 jar 판 겹침 점검 — 같은 라이브러리가 판만 다르게 두 개 이상 붙어 있는지 파일 이름으로 찾는다.
//
// 실행기는 프로젝트 src/main/webapp/WEB-INF/lib(docBase) 뒤에 의존 jar 폴더를 같은 /WEB-INF/lib 로 붙인다
// (base/conf/server.xml 의 PostResources). 판만 다른 두 jar 는 한 클래스로더에 함께 올라가고 어느 쪽 클래스가
// 먼저 읽힐지는 이클립스와 같다는 보장이 없다 — 이 실행기에서만 NoSuchMethodError · ClassCastException 이 난다.
// 이클립스 배포본에서 옮길 때는 이름이 완전히 같은 jar 만 거르고(eclipse.rs extra_jars), Maven 으로 받을 때는 거르지 않는다.
//
// 판은 이름의 마지막 「-숫자」부터로 본다 — spring-core-4.3.2.RELEASE → spring-core · 4.3.2.RELEASE,
// log4j-1.2-api-2.17.1 → log4j-1.2-api · 2.17.1. 판이 없으면(ojdbc8) 이름 전체가 이름이고 판은 비어 있다.
// jar 안의 pom.properties 는 읽지 않는다 — 이름을 바꿔 둔 jar 는 잡지 못한다.
//
// 2026-10-29~ : 알리기만 하던 것을 치울 수 있게 했다. 남길 판은 프로젝트 WEB-INF/lib 에 있으면 그 판, 없으면 가장 높은 판이다.
// 치우는 것은 의존 jar 폴더 쪽뿐이다 — 프로젝트 쪽은 소스 저장소의 파일이라 실행기가 지우지 않고 지울 파일 이름만 알린다.
// 치운 jar 는 지우지 않고 의존 jar 폴더 옆 <deps>.dup 로 옮긴다. /WEB-INF/lib 에도 컴파일 classpath(deps/*)에도 잡히지 않고 되돌릴 수 있다.

use std::cmp::Ordering;
use std::fs;
use std::path::{Path, PathBuf};

/// 같은 라이브러리의 다른 판 하나 — jars 는 (프로젝트 WEB-INF/lib 에 있나, 판, 파일 이름)
pub struct Dup {
    pub key: String,
    pub jars: Vec<(bool, String, String)>,
    pub keep: usize,         // 남길 jar (jars 의 자리)
    pub drop: Vec<String>,   // 치울 의존 jar 파일 — 남길 판과 판이 다른 것
    pub manual: Vec<String>, // 프로젝트 WEB-INF/lib 에서 사람이 지워야 하는 파일 — 프로젝트 안에서 판이 겹친다
}

/// 「이름 · 판」 으로 나눈다. 이름은 소문자.
pub fn split_version(file: &str) -> (String, String) {
    let stem = file.strip_suffix(".jar").or_else(|| file.strip_suffix(".JAR")).unwrap_or(file);
    let b = stem.as_bytes();
    let at = (1..b.len()).rev().find(|&k| b[k - 1] == b'-' && b[k].is_ascii_digit());
    match at {
        Some(k) => (stem[..k - 1].to_ascii_lowercase(), stem[k..].to_string()),
        None => (stem.to_ascii_lowercase(), String::new()),
    }
}

// 판에서 분류자(classifier) 꼬리를 뗀다 — netty 네이티브(…-linux-x86_64 · …-osx-aarch_64)처럼 판이 같고 플랫폼만 다른 jar 는 함께 두는 것이 맞다
fn core(ver: &str) -> String {
    const CLASSIFIER: [&str; 8] = ["linux", "osx", "macos", "windows", "natives", "sources", "javadoc", "tests"];
    let mut parts = Vec::new();
    for (k, seg) in ver.split('-').enumerate() {
        let low = seg.to_ascii_lowercase();
        if k > 0 && CLASSIFIER.iter().any(|c| low.starts_with(c)) {
            break;
        }
        parts.push(seg);
    }
    parts.join("-").to_ascii_lowercase()
}

// 판 비교 — 숫자 마디끼리 견준다 (2.11.0 > 2.9.8). 마디가 같으면 글자 순. 판이 없으면(ojdbc8.jar) 가장 낮다.
fn cmp_ver(a: &str, b: &str) -> Ordering {
    let nums = |v: &str| -> Vec<u64> { core(v).split(|c: char| !c.is_ascii_digit()).filter(|s| !s.is_empty()).map(|s| s.parse().unwrap_or(u64::MAX)).collect() };
    nums(a).cmp(&nums(b)).then_with(|| core(a).cmp(&core(b)))
}

// 남길 판 · 치울 판을 정한다 — 프로젝트 쪽이 있으면 그중 가장 높은 판, 없으면 전체에서 가장 높은 판을 남긴다.
// 남길 판과 분류자만 다른 jar(같은 판의 다른 플랫폼)는 함께 남긴다.
fn decide(d: &mut Dup) {
    let best = |mine_only: bool| {
        (0..d.jars.len()).filter(|&k| !mine_only || d.jars[k].0).max_by(|&x, &y| cmp_ver(&d.jars[x].1, &d.jars[y].1))
    };
    d.keep = best(true).or_else(|| best(false)).unwrap_or(0);
    let kept = core(&d.jars[d.keep].1);
    for (mine, ver, file) in &d.jars {
        if core(ver) != kept {
            if *mine { d.manual.push(file.clone()) } else { d.drop.push(file.clone()) }
        }
    }
}

fn jars(dir: &Path) -> Vec<String> {
    let mut v: Vec<String> = fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.file_name().to_string_lossy().to_string())
        .filter(|n| n.to_ascii_lowercase().ends_with(".jar"))
        .collect();
    v.sort();
    v
}

/// own(프로젝트 WEB-INF/lib)과 deps(의존 jar 폴더)를 합쳐 판이 둘 이상인 라이브러리. 이름 순.
pub fn duplicates(own: &Path, deps: &Path) -> Vec<Dup> {
    let own_names = jars(own);
    let own_low: Vec<String> = own_names.iter().map(|n| n.to_ascii_lowercase()).collect();
    // 이름이 완전히 같으면 docBase 쪽이 이기고 의존 jar 쪽은 가려진다 — 겹침이 아니다
    let all = own_names
        .into_iter()
        .map(|n| (true, n))
        .chain(jars(deps).into_iter().filter(|n| !own_low.contains(&n.to_ascii_lowercase())).map(|n| (false, n)));
    let mut groups: Vec<Dup> = Vec::new();
    for (mine, file) in all {
        let (key, ver) = split_version(&file);
        match groups.iter_mut().find(|g| g.key == key) {
            Some(g) => g.jars.push((mine, ver, file)),
            None => groups.push(Dup { key, jars: vec![(mine, ver, file)], keep: 0, drop: vec![], manual: vec![] }),
        }
    }
    groups.retain(|g| {
        let mut cores: Vec<String> = g.jars.iter().map(|j| core(&j.1)).collect();
        cores.sort();
        cores.dedup();
        cores.len() > 1
    });
    groups.iter_mut().for_each(decide);
    groups.sort_by(|a, b| a.key.cmp(&b.key));
    groups
}

fn label(mine: bool, ver: &str, file: &str) -> String {
    format!("{} {}", if mine { "프로젝트" } else { "의존 jar" }, if ver.is_empty() { file } else { ver })
}

/// 라이브러리 하나의 할 일 — 「commons-io — 남김 프로젝트 2.4 · 치움 의존 jar 2.11.0」
pub fn plan_line(d: &Dup) -> String {
    let (mine, ver, file) = &d.jars[d.keep];
    let mut parts = vec![format!("남김 {}", label(*mine, ver, file))];
    for (m, v, f) in &d.jars {
        if d.drop.contains(f) {
            parts.push(format!("치움 {}", label(*m, v, f)));
        }
    }
    if !d.manual.is_empty() {
        parts.push(format!("프로젝트 WEB-INF/lib 에서 지울 것 {}", d.manual.join(", ")));
    }
    format!("{} — {}", d.key, parts.join(" · "))
}

/// 점검 줄의 설명 — 첫 줄은 요약(알림이 쓴다), 그다음 라이브러리마다 한 줄, 끝에 까닭과 푸는 법
pub fn detail(dups: &[Dup]) -> String {
    const SHOW: usize = 8;
    let names: Vec<&str> = dups.iter().take(3).map(|d| d.key.as_str()).collect();
    let mut lines = vec![format!("{}종 · {}{}", dups.len(), names.join(" · "), if dups.len() > 3 { " 외" } else { "" })];
    lines.extend(dups.iter().take(SHOW).map(plan_line));
    if dups.len() > SHOW {
        lines.push(format!("외 {}종", dups.len() - SHOW));
    }
    lines.push("두 판이 함께 올라가 이클립스와 클래스를 읽는 순서가 달라지므로, 이 실행기에서만 NoSuchMethodError · ClassCastException 이 날 수 있습니다".into());
    let movable: usize = dups.iter().map(|d| d.drop.len()).sum();
    if movable > 0 {
        lines.push(format!("「겹친 판 치우기」를 누르면 「치움」 jar {movable}개를 의존 jar 폴더 옆 deps.dup 로 옮깁니다. 다시 받아도 다시 치우고, 「되돌리기」로 돌려놓습니다"));
    }
    if dups.iter().any(|d| !d.manual.is_empty()) {
        lines.push("「지울 것」은 프로젝트 소스 폴더의 jar 라 실행기가 손대지 않습니다. 소스 저장소에서 지우거나 pom.xml 의 판을 맞추세요".into());
    }
    lines.join("\n")
}

/// 치운 jar 를 두는 곳 — 의존 jar 폴더 옆 <이름>.dup (deps.new · deps.old 와 같은 자리)
pub fn aside_dir(deps: &Path) -> PathBuf {
    deps.with_file_name(format!("{}.dup", deps.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| "deps".into())))
}

/// 치워 둔 jar 파일 이름
pub fn aside_jars(deps: &Path) -> Vec<String> {
    jars(&aside_dir(deps))
}

/// 「치움」 jar 를 모두 aside_dir 로 옮긴다. 하나라도 못 옮기면 옮긴 것을 되돌리고 실패한다. 옮긴 수를 돌려준다.
pub fn sweep(own: &Path, deps: &Path) -> Result<usize, String> {
    let files: Vec<String> = duplicates(own, deps).into_iter().flat_map(|d| d.drop).collect();
    if files.is_empty() {
        return Ok(0);
    }
    let aside = aside_dir(deps);
    fs::create_dir_all(&aside).map_err(|e| format!("폴더를 만들지 못했습니다 ({}): {e}", aside.display()))?;
    for (k, f) in files.iter().enumerate() {
        // 같은 이름이 남아 있으면 지난번에 치운 사본이다 — 윈도에서는 대상이 있으면 옮기지 못한다
        let _ = fs::remove_file(aside.join(f));
        if let Err(e) = fs::rename(deps.join(f), aside.join(f)) {
            for g in &files[..k] {
                let _ = fs::rename(aside.join(g), deps.join(g));
            }
            return Err(format!("{f} 를 옮기지 못했습니다 (톰캣이 켜져 있으면 끄세요): {e}"));
        }
    }
    Ok(files.len())
}

/// 치워 둔 jar 를 의존 jar 폴더로 돌려놓는다. 다시 받아 이미 같은 이름이 있으면 치운 사본만 지운다. 돌려놓은 수를 돌려준다.
pub fn restore(deps: &Path) -> Result<usize, String> {
    let aside = aside_dir(deps);
    let names = jars(&aside);
    fs::create_dir_all(deps).map_err(|e| format!("폴더를 만들지 못했습니다 ({}): {e}", deps.display()))?;
    let mut n = 0;
    for f in &names {
        if deps.join(f).exists() {
            let _ = fs::remove_file(aside.join(f));
            continue;
        }
        fs::rename(aside.join(f), deps.join(f)).map_err(|e| format!("{f} 를 돌려놓지 못했습니다 (톰캣이 켜져 있으면 끄세요): {e}"))?;
        n += 1;
    }
    let _ = fs::remove_dir(&aside); // 비었을 때만 지워진다
    Ok(n)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 이름에서_판을_뗀다() {
        assert_eq!(split_version("spring-core-4.3.2.RELEASE.jar"), ("spring-core".into(), "4.3.2.RELEASE".into()));
        assert_eq!(split_version("log4j-1.2-api-2.17.1.jar"), ("log4j-1.2-api".into(), "2.17.1".into()));
        assert_eq!(split_version("commons-lang3-3.12.0.jar"), ("commons-lang3".into(), "3.12.0".into()));
        assert_eq!(split_version("egov-common-1.0.0-SNAPSHOT.jar"), ("egov-common".into(), "1.0.0-SNAPSHOT".into()));
        assert_eq!(split_version("Ojdbc8.jar"), ("ojdbc8".into(), String::new()));
        assert_eq!(split_version("bcprov-jdk15on-1.70.jar"), ("bcprov-jdk15on".into(), "1.70".into()));
    }

    #[test]
    fn 판이_다른_같은_라이브러리만_잡는다() {
        let root = std::env::temp_dir().join(format!("tomcatrun-jarcheck-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let (own, deps) = (root.join("own"), root.join("deps"));
        fs::create_dir_all(&own).unwrap();
        fs::create_dir_all(&deps).unwrap();
        for n in ["commons-io-2.4.jar", "same-1.0.jar", "log4j-1.2.17.jar"] {
            fs::write(own.join(n), "").unwrap();
        }
        for n in [
            "commons-io-2.11.0.jar",
            "same-1.0.jar",                                   // 이름이 같으면 가려진다
            "log4j-1.2-api-2.17.1.jar",                       // 다른 라이브러리
            "commons-lang-2.6.jar",
            "commons-lang3-3.12.0.jar",                       // 다른 라이브러리
            "netty-transport-native-epoll-4.1.86.Final-linux-x86_64.jar",
            "netty-transport-native-epoll-4.1.86.Final-linux-aarch_64.jar", // 플랫폼만 다르다
            "jackson-databind-2.9.8.jar",
            "jackson-databind-2.12.3.jar",
            "ojdbc8.jar",
            "ojdbc8-19.3.0.0.jar",                            // 판 없는 이름도 같은 라이브러리
        ] {
            fs::write(deps.join(n), "").unwrap();
        }
        let d = duplicates(&own, &deps);
        let keys: Vec<&str> = d.iter().map(|x| x.key.as_str()).collect();
        assert_eq!(keys, ["commons-io", "jackson-databind", "ojdbc8"]);
        assert_eq!(d[0].jars.iter().map(|j| (j.0, j.1.as_str())).collect::<Vec<_>>(), [(true, "2.4"), (false, "2.11.0")]);
        let text = detail(&d);
        assert!(text.starts_with("3종 · commons-io · jackson-databind · ojdbc8\n"));
        assert!(text.contains("commons-io — 남김 프로젝트 2.4 · 치움 의존 jar 2.11.0"));
        assert!(text.contains("jackson-databind — 남김 의존 jar 2.12.3 · 치움 의존 jar 2.9.8"));
        assert!(text.contains("ojdbc8 — 남김 의존 jar 19.3.0.0 · 치움 의존 jar ojdbc8.jar"));
        assert!(text.contains("「치움」 jar 3개"));
        assert!(!text.contains("지울 것"));
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn 판은_숫자_마디로_견준다() {
        assert_eq!(cmp_ver("2.11.0", "2.9.8"), Ordering::Greater);
        assert_eq!(cmp_ver("4.1.86.Final-linux-x86_64", "4.1.9.Final"), Ordering::Greater);
        assert_eq!(cmp_ver("", "1.0"), Ordering::Less);
        assert_eq!(cmp_ver("3.10", "3.9"), Ordering::Greater);
    }

    #[test]
    fn 프로젝트_판을_남기고_프로젝트_안의_겹침은_알리기만_한다() {
        let root = std::env::temp_dir().join(format!("tomcatrun-jarcheck-sweep-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let (own, deps) = (root.join("own"), root.join("deps"));
        fs::create_dir_all(&own).unwrap();
        fs::create_dir_all(&deps).unwrap();
        for n in ["poi-3.9.jar", "poi-3.10.jar", "commons-io-2.4.jar"] {
            fs::write(own.join(n), "").unwrap();
        }
        for n in ["poi-3.17.jar", "commons-io-2.11.0.jar", "gson-2.8.0.jar"] {
            fs::write(deps.join(n), "").unwrap();
        }
        let d = duplicates(&own, &deps);
        // 의존 jar 쪽이 더 높아도 프로젝트 판을 남긴다
        assert_eq!(plan_line(&d[0]), "commons-io — 남김 프로젝트 2.4 · 치움 의존 jar 2.11.0");
        assert_eq!(plan_line(&d[1]), "poi — 남김 프로젝트 3.10 · 치움 의존 jar 3.17 · 프로젝트 WEB-INF/lib 에서 지울 것 poi-3.9.jar");
        assert!(detail(&d).contains("실행기가 손대지 않습니다"));

        assert_eq!(sweep(&own, &deps).unwrap(), 2);
        assert_eq!(aside_jars(&deps), ["commons-io-2.11.0.jar", "poi-3.17.jar"]);
        assert!(deps.join("gson-2.8.0.jar").exists() && !deps.join("poi-3.17.jar").exists());
        assert!(own.join("poi-3.9.jar").exists(), "프로젝트 jar 는 건드리지 않는다");
        // 치운 뒤에도 프로젝트 안의 겹침은 남는다
        let left = duplicates(&own, &deps);
        assert_eq!(left.len(), 1);
        assert!(left[0].drop.is_empty() && left[0].manual == ["poi-3.9.jar"]);

        assert_eq!(restore(&deps).unwrap(), 2);
        assert!(deps.join("poi-3.17.jar").exists() && !aside_dir(&deps).exists());
        let _ = fs::remove_dir_all(&root);
    }

    // 실제 폴더를 읽어 보인다 (읽기만): TOMCATRUN_TEST_JARS=<WEB-INF/lib>;<의존 jar 폴더> cargo test jarcheck -- --ignored --nocapture
    #[test]
    #[ignore]
    fn 실제_폴더() {
        let Ok(v) = std::env::var("TOMCATRUN_TEST_JARS") else { return };
        let (own, deps) = v.split_once(';').unwrap_or(("", &v));
        println!("{}", detail(&duplicates(Path::new(own), Path::new(deps))));
    }
}
