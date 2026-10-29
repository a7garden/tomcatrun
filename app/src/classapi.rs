// 클래스의 바깥 모양 — 저장한 자바의 상수 값 · 메서드 · 필드 시그니처가 바뀌었는지 가리고, 그것을 쓰는 소스를 찾는다.
//
// 저장마다 바뀐 파일만 컴파일한다. 그런데 자바 컴파일러는 다른 클래스의 static final 상수를 값 그대로 박아 넣고,
// 메서드는 이름 · 매개변수 꼴로 부른다. 그래서 상수 값이나 시그니처를 바꾸면 쓰는 쪽 클래스에는 옛 값 · 옛 꼴이 남는다.
// 컴파일 오류 없이 옛 값으로 돌고 재시작해도 그대로였다 — 전체 컴파일만 풀었다.
// 2026.10.29 : 컴파일 앞뒤로 모양을 견주어, 바뀌었으면 쓰는 소스를 함께 컴파일한다 (main.rs 의 run_compile).
//
// 쓰는 곳은 소스에 클래스 이름이 낱말로 나오는지로 찾는다. 주석 속 이름도 걸리지만 더 컴파일할 뿐이라 해가 없다.

use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

/// 소스 하나(src/main/java 기준 「a/b/C.java」)가 만든 클래스 파일 — C.class 와 C$*.class.
/// 익명 · 지역 클래스(C$1 · C$1Local)는 anon 이 참일 때만 — 바깥에서 부를 수 없어 모양을 견줄 까닭이 없다.
pub fn class_files(classes: &Path, rel: &str, anon: bool) -> Vec<PathBuf> {
    let rel = rel.replace('\\', "/");
    let Some(stem) = rel.strip_suffix(".java") else { return Vec::new() };
    let (dir, name) = match stem.rfind('/') {
        Some(k) => (classes.join(&stem[..k]), &stem[k + 1..]),
        None => (classes.to_path_buf(), stem),
    };
    let Ok(rd) = fs::read_dir(&dir) else { return Vec::new() };
    let inner = format!("{name}$");
    let mut out: Vec<PathBuf> = rd
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| {
            let Some(f) = p.file_name().and_then(|f| f.to_str()) else { return false };
            let Some(c) = f.strip_suffix(".class") else { return false };
            if c == name {
                return true;
            }
            let Some(rest) = c.strip_prefix(&inner) else { return false };
            anon || !rest.split('$').any(|part| part.starts_with(|ch: char| ch.is_ascii_digit()))
        })
        .collect();
    out.sort();
    out
}

/// 소스마다 지금 클래스들의 모양. 클래스 파일이 없으면(처음 컴파일) 빈 글자.
pub fn snapshot(classes: &Path, rels: &[String]) -> HashMap<String, String> {
    rels.iter()
        .map(|rel| {
            let shape: Vec<String> = class_files(classes, rel, false)
                .iter()
                .filter_map(|f| fs::read(f).ok().and_then(|b| shape(&b)).map(|s| format!("{}\n{s}", f.file_name().unwrap_or_default().to_string_lossy())))
                .collect();
            (rel.clone(), shape.join("\n"))
        })
        .collect()
}

/// 모양이 바뀐 소스의 클래스 이름 (「a/b/C.java」 → 「C」). 처음 생긴 클래스는 쓰던 곳이 없으므로 뺀다.
pub fn changed(before: &HashMap<String, String>, after: &HashMap<String, String>) -> Vec<String> {
    let mut out: Vec<String> = before
        .iter()
        .filter(|(rel, old)| !old.is_empty() && after.get(*rel) != Some(*old))
        .filter_map(|(rel, _)| rel.rsplit('/').next().and_then(|f| f.strip_suffix(".java")).map(str::to_string))
        .collect();
    out.sort();
    out.dedup();
    out
}

/// 소스 폴더에서 이 이름들 가운데 하나라도 낱말로 나오는 .java (skip 에 든 것은 뺀다)
pub fn users(src: &Path, names: &[String], skip: &HashSet<PathBuf>) -> Vec<PathBuf> {
    fn walk(dir: &Path, names: &[String], skip: &HashSet<PathBuf>, out: &mut Vec<PathBuf>) {
        let Ok(rd) = fs::read_dir(dir) else { return };
        for e in rd.filter_map(Result::ok) {
            let path = e.path();
            if path.is_dir() {
                walk(&path, names, skip, out);
            } else if path.extension().map_or(false, |x| x == "java") && !skip.contains(&path) {
                // 인코딩은 서버마다 다르지만 클래스 이름은 ASCII 라 바이트로 찾는다
                let Ok(b) = fs::read(&path) else { continue };
                if names.iter().any(|n| has_word(&b, n.as_bytes())) {
                    out.push(path);
                }
            }
        }
    }
    let mut out = Vec::new();
    if !names.is_empty() {
        walk(src, names, skip, &mut out);
    }
    out.sort();
    out
}

/// 지운 소스가 남긴 클래스 파일을 지운다 (익명 클래스까지). 지운 개수.
pub fn remove(classes: &Path, rel: &str) -> usize {
    class_files(classes, rel, true).iter().filter(|f| fs::remove_file(f).is_ok()).count()
}

fn has_word(text: &[u8], word: &[u8]) -> bool {
    // 자바 식별자 글자 — 한글 같은 비 ASCII 바이트는 경계로 본다 (더 걸려도 더 컴파일할 뿐이다)
    let ident = |c: u8| c.is_ascii_alphanumeric() || c == b'_' || c == b'$';
    if word.is_empty() || text.len() < word.len() {
        return false;
    }
    (0..=text.len() - word.len()).any(|k| {
        &text[k..k + word.len()] == word && (k == 0 || !ident(text[k - 1])) && text.get(k + word.len()).map_or(true, |&c| !ident(c))
    })
}

/// 클래스 파일에서 바깥에서 보이는 것만 줄로 — 상위 클래스 · 인터페이스, private 이 아닌 필드(상수면 값까지) · 메서드 시그니처.
/// 메서드 본문만 고쳤으면 그대로다. 컴파일러가 만든 것(synthetic)은 뺀다 — 람다 · 안쪽 클래스 접근용이 본문을 따라 바뀐다.
pub fn shape(b: &[u8]) -> Option<String> {
    let mut r = Reader { b, at: 0 };
    if r.u4()? != 0xCAFE_BABE {
        return None;
    }
    r.skip(4)?; // minor · major
    let n = r.u2()? as usize;
    let mut pool: Vec<Const> = vec![Const::Other; n.max(1)];
    let mut k = 1;
    while k < n {
        let tag = r.u1()?;
        pool[k] = match tag {
            1 => {
                let len = r.u2()? as usize;
                Const::Utf8(String::from_utf8_lossy(r.take(len)?).into_owned())
            }
            3 => Const::Num(format!("I{}", r.u4()? as i32)),
            4 => Const::Num(format!("F{:08x}", r.u4()?)),
            5 | 6 => {
                let v = ((r.u4()? as u64) << 32) | r.u4()? as u64;
                Const::Num(format!("{}{v:016x}", if tag == 5 { 'J' } else { 'D' }))
            }
            7 | 8 | 16 | 19 | 20 => Const::Ref(r.u2()?),
            9 | 10 | 11 | 12 | 17 | 18 => {
                r.skip(4)?;
                Const::Other
            }
            15 => {
                r.skip(3)?;
                Const::Other
            }
            _ => return None,
        };
        // long · double 은 두 칸을 쓴다
        k += if tag == 5 || tag == 6 { 2 } else { 1 };
    }
    let utf = |i: u16| match pool.get(i as usize) {
        Some(Const::Utf8(s)) => s.clone(),
        _ => String::new(),
    };
    let class = |i: u16| match pool.get(i as usize) {
        Some(Const::Ref(u)) => utf(*u),
        _ => String::new(),
    };
    let value = |i: u16| match pool.get(i as usize) {
        Some(Const::Num(v)) => v.clone(),
        Some(Const::Ref(u)) => format!("S{}", utf(*u)),
        _ => String::new(),
    };
    let mut lines = Vec::new();
    r.skip(2)?; // access
    r.skip(2)?; // this
    lines.push(format!("extends {}", class(r.u2()?)));
    for _ in 0..r.u2()? {
        lines.push(format!("implements {}", class(r.u2()?)));
    }
    const PRIVATE: u16 = 0x0002;
    const STATIC: u16 = 0x0008;
    const SYNTHETIC: u16 = 0x1000;
    for kind in ["field", "method"] {
        for _ in 0..r.u2()? {
            let (access, name, desc) = (r.u2()?, r.u2()?, r.u2()?);
            let mut constant = String::new();
            for _ in 0..r.u2()? {
                let (attr, len) = (r.u2()?, r.u4()? as usize);
                if kind == "field" && len == 2 && utf(attr) == "ConstantValue" {
                    constant = value(r.u2()?);
                } else {
                    r.skip(len)?;
                }
            }
            if access & (PRIVATE | SYNTHETIC) != 0 {
                continue;
            }
            let stat = if access & STATIC != 0 { "static " } else { "" };
            let eq = if constant.is_empty() { String::new() } else { format!(" = {constant}") };
            lines.push(format!("{kind} {stat}{} {}{eq}", utf(name), utf(desc)));
        }
    }
    lines.sort();
    Some(lines.join("\n"))
}

#[derive(Clone)]
enum Const {
    Utf8(String),
    Num(String),
    Ref(u16), // Class · String · MethodType · Module · Package — 가리키는 Utf8
    Other,
}

struct Reader<'a> {
    b: &'a [u8],
    at: usize,
}

impl<'a> Reader<'a> {
    fn take(&mut self, n: usize) -> Option<&'a [u8]> {
        let s = self.b.get(self.at..self.at.checked_add(n)?)?;
        self.at += n;
        Some(s)
    }
    fn skip(&mut self, n: usize) -> Option<()> {
        self.take(n).map(|_| ())
    }
    fn u1(&mut self) -> Option<u8> {
        self.take(1).map(|s| s[0])
    }
    fn u2(&mut self) -> Option<u16> {
        self.take(2).map(|s| u16::from_be_bytes([s[0], s[1]]))
    }
    fn u4(&mut self) -> Option<u32> {
        self.take(4).map(|s| u32::from_be_bytes([s[0], s[1], s[2], s[3]]))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // 상수 하나(static final int X = value)와 메서드 하나(접근 플래그 m_access, 이름 method)인 클래스 A
    fn class_bytes(value: i32, m_access: u16, method: &str) -> Vec<u8> {
        let mut b = vec![0xCA, 0xFE, 0xBA, 0xBE, 0, 0, 0, 52];
        let utf = |b: &mut Vec<u8>, s: &str| {
            b.push(1);
            b.extend((s.len() as u16).to_be_bytes());
            b.extend(s.as_bytes());
        };
        b.extend(11u16.to_be_bytes()); // 상수 풀 1..=10
        utf(&mut b, "A"); // 1
        b.extend([7, 0, 1]); // 2 Class A
        utf(&mut b, "java/lang/Object"); // 3
        b.extend([7, 0, 3]); // 4 Class Object
        utf(&mut b, "X"); // 5
        utf(&mut b, "I"); // 6
        utf(&mut b, "ConstantValue"); // 7
        b.push(3);
        b.extend(value.to_be_bytes()); // 8 Integer
        utf(&mut b, method); // 9
        utf(&mut b, "()V"); // 10
        b.extend([0, 0x21, 0, 2, 0, 4, 0, 0]); // access · this · super · 인터페이스 0
        b.extend([0, 1, 0, 0x19, 0, 5, 0, 6, 0, 1, 0, 7, 0, 0, 0, 2, 0, 8]); // 필드 1 (public static final X = #8)
        b.extend([0, 1]);
        b.extend(m_access.to_be_bytes());
        b.extend([0, 9, 0, 10, 0, 0]); // 메서드 1
        b.extend([0, 0]); // 속성 0
        b
    }

    #[test]
    fn shape_sees_constant_and_signature() {
        let a = shape(&class_bytes(5, 0x0001, "run")).expect("읽힘");
        assert!(a.contains("extends java/lang/Object"));
        assert!(a.contains("field static X I = I5"), "{a}");
        assert!(a.contains("method run ()V"), "{a}");
        assert_eq!(shape(&class_bytes(5, 0x0001, "run")), Some(a.clone()));
        assert_ne!(shape(&class_bytes(6, 0x0001, "run")), Some(a.clone()));
        assert_ne!(shape(&class_bytes(5, 0x0001, "go")), Some(a.clone()));
        // private · synthetic 메서드는 바깥에서 안 보인다
        assert!(!shape(&class_bytes(5, 0x0002, "run")).unwrap().contains("run"));
        assert!(!shape(&class_bytes(5, 0x1001, "run")).unwrap().contains("run"));
        assert_eq!(shape(b"not a class"), None);
        assert_eq!(shape(&class_bytes(5, 1, "run")[..40]), None);
    }

    #[test]
    fn reads_real_classes() {
        // 이 저장소에서 컴파일해 둔 클래스가 있으면 그것으로 — 없으면(CI) 건너뛴다
        fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
            let Ok(rd) = fs::read_dir(dir) else { return };
            for e in rd.filter_map(Result::ok).map(|e| e.path()) {
                if e.is_dir() {
                    walk(&e, out);
                } else if e.extension().map_or(false, |x| x == "class") {
                    out.push(e);
                }
            }
        }
        let mut files = Vec::new();
        walk(&Path::new(env!("CARGO_MANIFEST_DIR")).join("../classes"), &mut files);
        let bad: Vec<_> = files.iter().filter(|f| fs::read(f).ok().and_then(|b| shape(&b)).is_none()).collect();
        assert!(bad.is_empty(), "{}개 중 못 읽은 클래스: {:?}", files.len(), bad.iter().take(5).collect::<Vec<_>>());
    }

    #[test]
    fn word_match_needs_boundaries() {
        assert!(has_word(b"import a.b.Codes;", b"Codes"));
        assert!(has_word(b"x = Codes.MAX;", b"Codes"));
        assert!(has_word(b"Codes", b"Codes"));
        assert!(!has_word(b"MyCodes.MAX", b"Codes"));
        assert!(!has_word(b"CodesUtil", b"Codes"));
        assert!(!has_word(b"Codes_old", b"Codes"));
        assert!(!has_word(b"Code", b"Codes"));
    }

    #[test]
    fn finds_classes_users_and_changes() {
        let root = std::env::temp_dir().join(format!("tomcatrun-classapi-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let (src, classes) = (root.join("src"), root.join("classes"));
        fs::create_dir_all(src.join("a")).unwrap();
        fs::create_dir_all(classes.join("a")).unwrap();
        fs::write(src.join("a/Codes.java"), "class Codes { static final int MAX = 5; }").unwrap();
        fs::write(src.join("a/Use.java"), "class Use { int x = Codes.MAX; }").unwrap();
        fs::write(src.join("a/Other.java"), "class Other { /* MyCodes */ }").unwrap();
        for f in ["Codes.class", "Codes$Kind.class", "Codes$1.class", "CodesUtil.class"] {
            fs::write(classes.join("a").join(f), class_bytes(5, 1, "run")).unwrap();
        }
        let names = |v: Vec<PathBuf>| v.iter().map(|p| p.file_name().unwrap().to_string_lossy().into_owned()).collect::<Vec<_>>();
        assert_eq!(names(class_files(&classes, "a/Codes.java", false)), ["Codes$Kind.class", "Codes.class"]);
        assert_eq!(names(class_files(&classes, "a/Codes.java", true)).len(), 3);

        let rels = vec!["a/Codes.java".to_string(), "a/New.java".to_string()];
        let before = snapshot(&classes, &rels);
        fs::write(classes.join("a/New.class"), class_bytes(1, 1, "run")).unwrap();
        assert!(changed(&before, &snapshot(&classes, &rels)).is_empty(), "새 클래스 · 그대로인 클래스는 바뀐 것이 아니다");
        fs::write(classes.join("a/Codes.class"), class_bytes(7, 1, "run")).unwrap();
        assert_eq!(changed(&before, &snapshot(&classes, &rels)), ["Codes"]);

        let skip = HashSet::from([src.join("a").join("Codes.java")]);
        assert_eq!(names(users(&src, &["Codes".into()], &skip)), ["Use.java"]);

        assert_eq!(remove(&classes, "a/Codes.java"), 3);
        assert!(classes.join("a/CodesUtil.class").exists(), "이름이 이어지는 다른 클래스는 남긴다");
        let _ = fs::remove_dir_all(&root);
    }
}
