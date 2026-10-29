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

use std::fs;
use std::path::Path;

/// 같은 라이브러리의 다른 판 하나 — jars 는 (프로젝트 WEB-INF/lib 에 있나, 판, 파일 이름)
pub struct Dup {
    pub key: String,
    pub jars: Vec<(bool, String, String)>,
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
            None => groups.push(Dup { key, jars: vec![(mine, ver, file)] }),
        }
    }
    groups.retain(|g| {
        let mut cores: Vec<String> = g.jars.iter().map(|j| core(&j.1)).collect();
        cores.sort();
        cores.dedup();
        cores.len() > 1
    });
    groups.sort_by(|a, b| a.key.cmp(&b.key));
    groups
}

/// 점검 줄의 설명 — 첫 줄은 요약(알림이 쓴다), 그다음 라이브러리마다 한 줄, 끝에 푸는 법
pub fn detail(dups: &[Dup]) -> String {
    const SHOW: usize = 8;
    let names: Vec<&str> = dups.iter().take(3).map(|d| d.key.as_str()).collect();
    let mut lines = vec![format!("{}종 · {}{}", dups.len(), names.join(" · "), if dups.len() > 3 { " 외" } else { "" })];
    for d in dups.iter().take(SHOW) {
        let vers: Vec<String> = d
            .jars
            .iter()
            .map(|(mine, ver, file)| format!("{} {}", if *mine { "프로젝트" } else { "의존 jar" }, if ver.is_empty() { file.as_str() } else { ver.as_str() }))
            .collect();
        lines.push(format!("{} — {}", d.key, vers.join(" · ")));
    }
    if dups.len() > SHOW {
        lines.push(format!("외 {}종", dups.len() - SHOW));
    }
    lines.push("이클립스와 클래스를 읽는 순서가 달라 이 실행기에서만 NoSuchMethodError · ClassCastException 이 날 수 있습니다. 한쪽 jar 를 지우거나 pom.xml 의 판을 맞추세요".into());
    lines.join("\n")
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
        assert!(text.contains("commons-io — 프로젝트 2.4 · 의존 jar 2.11.0"));
        assert!(text.contains("ojdbc8 — 의존 jar 19.3.0.0 · 의존 jar ojdbc8.jar"));
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
