// Commit message — 언어 모델 CLI 로 제목 한 줄 만들기 · 팀 규칙 검사 (2026.10.29~).
//
// 「설정 › 경로 › Commit message 생성 명령」(예: claude -p)이 비어 있으면 만들지 않는다. 기본은 비어 있다.
// 채우면 「자동 생성」을 누를 때 고른 파일의 Diff 가 그 명령으로 넘어간다 — 소스가 그 CLI 의 서비스로 나간다.
//   프롬프트는 인자가 아니라 stdin 으로 넘긴다. Diff 에 든 따옴표 · & · % 가 cmd 에서 해석되지 않게 하려는 것이다.
//   명령 자체는 cmd /c 로 적힌 그대로 부른다 (npm 으로 깐 CLI 는 .cmd 라 cmd 를 거쳐야 한다 · 편집기 칸과 같다).
// 규칙(서버마다 「설정 › 버전 관리」)은 막지 않고 알리기만 한다. 어긴 채 Commit 하려면 확인 창에서 한 번 더 고른다.
//
// 목차 : 규칙(Rule · validate · check · issue_of) · 프롬프트(FileDiff · diff_text · build_prompt) · 실행(run · clean)

use std::os::windows::process::CommandExt;
use std::process::Command;
use std::time::Duration;

use regex::Regex;

use crate::svn::{decode, Diff};
use crate::svncli;

// CLI 가 로그인을 묻거나 멈추면 이만큼 기다린 뒤 끊는다
const GEN_LIMIT: Duration = Duration::from_secs(90);
// 파일 하나 · 전체 Diff 상한 — 프롬프트가 지나치게 커지거나 CLI 가 거절하지 않게
const FILE_MAX: usize = 8 * 1024;
const TOTAL_MAX: usize = 60 * 1024;

/* ---------- 규칙 ---------- */

/// 서버 하나의 메시지 규칙 — 빈 칸은 쓰지 않는다
#[derive(Default)]
pub struct Rule {
    pub text: String,            // 모델에게 줄 설명 (예: 이슈 키 뒤에 한 줄 평서문)
    pub example: String,         // 「Commit message 예시」 — 메시지 칸의 흐린 글과 같은 값
    pub issue_pattern: String,   // 브랜치 · 작업 폴더 이름에서 이슈 번호를 찾는 정규식
    pub subject_pattern: String, // 제목이 맞아야 하는 정규식
    pub subject_max: u32,        // 제목 최대 글자 수 (0 이면 보지 않는다)
}

fn compile(what: &str, pat: &str) -> Result<Option<Regex>, String> {
    if pat.trim().is_empty() {
        return Ok(None);
    }
    Regex::new(pat.trim()).map(Some).map_err(|e| format!("{what} 정규식이 올바르지 않습니다: {e}"))
}

/// 저장하기 전에 정규식이 맞는지 본다 — 틀린 채 저장되면 Commit 할 때마다 검사가 실패한다
pub fn validate(r: &Rule) -> Result<(), String> {
    compile("이슈 번호 찾기", &r.issue_pattern)?;
    compile("제목 규칙", &r.subject_pattern)?;
    Ok(())
}

/// 규칙에 어긋난 까닭. 비면 맞다. 제목은 첫 줄이고 길이는 바이트가 아니라 글자 수로 센다 (한글 제목이 세 배로 잡히지 않게).
pub fn check(r: &Rule, msg: &str) -> Vec<String> {
    let subject = msg.trim().lines().next().unwrap_or("").trim();
    let mut out = Vec::new();
    if subject.is_empty() {
        out.push("Commit message 가 비었습니다.".into());
        return out;
    }
    match compile("제목 규칙", &r.subject_pattern) {
        Ok(Some(re)) if !re.is_match(subject) => out.push(format!("제목 규칙에 맞지 않습니다 ({})", r.subject_pattern.trim())),
        Err(e) => out.push(e),
        _ => {}
    }
    let n = subject.chars().count();
    if r.subject_max > 0 && n > r.subject_max as usize {
        out.push(format!("제목이 {}자를 넘습니다 ({n}자)", r.subject_max));
    }
    out
}

/// 브랜치(또는 작업 폴더 · 저장소 경로)에서 이슈 번호 하나
pub fn issue_of(pattern: &str, text: &str) -> Option<String> {
    let re = compile("이슈 번호 찾기", pattern).ok()??;
    re.find(text).map(|m| m.as_str().to_string())
}

/* ---------- 프롬프트 ---------- */

/// 고른 파일 하나 — diff 가 없으면(바이너리 · 너무 큼 · 폴더) 이름만 넘긴다
pub struct FileDiff {
    pub path: String,
    pub label: String, // Modified · Added · Deleted …
    pub diff: Option<String>,
}

/// 화면용 Diff 를 통합 diff 꼴 글로
pub fn diff_text(d: &Diff) -> Option<String> {
    if d.note.is_some() && d.lines.is_empty() {
        return None;
    }
    let mut s = String::new();
    for l in &d.lines {
        let mark = match l.t {
            "add" => "+",
            "del" => "-",
            "hunk" => "",
            _ => " ",
        };
        s.push_str(mark);
        s.push_str(&l.s);
        s.push('\n');
    }
    Some(s)
}

// 글자 경계에서 자른다 (바이트로 자르면 한글 가운데서 끊겨 무너진다)
fn cut(s: &str, max: usize) -> &str {
    if s.len() <= max {
        return s;
    }
    let mut i = max;
    while !s.is_char_boundary(i) {
        i -= 1;
    }
    &s[..i]
}

pub fn build_prompt(r: &Rule, issue: Option<&str>, branch: &str, files: &[FileDiff]) -> String {
    let mut p = String::from(
        "아래 변경을 설명하는 버전 관리 Commit message 의 제목을 한 줄로 써라.\n\
         - 제목 한 줄만 답한다. 따옴표 · 머리말 · 설명 · 본문 · 마크다운을 붙이지 않는다.\n\
         - 파일 이름을 늘어놓지 말고, 이 변경으로 무엇이 달라지는지를 쓴다.\n\
         - 아래 팀 규칙과 예시의 말투 · 언어를 따른다.\n",
    );
    let mut line = |k: &str, v: &str| {
        if !v.trim().is_empty() {
            p.push_str(&format!("[{k}] {}\n", v.trim()));
        }
    };
    line("팀 규칙", &r.text);
    line("예시", &r.example);
    line("제목 형식 (정규식)", &r.subject_pattern);
    if r.subject_max > 0 {
        line("최대 길이", &format!("{}자", r.subject_max));
    }
    line("이슈 번호 (규칙이 이슈 번호를 요구하면 이것을 쓴다)", issue.unwrap_or(""));
    line("브랜치 · 저장소 위치", branch);
    p.push_str("\n[바뀐 파일]\n");
    for f in files {
        p.push_str(&format!("- {} {}\n", f.label, f.path));
    }
    p.push_str("\n[Diff]\n");
    let mut used = 0;
    for f in files {
        let Some(d) = &f.diff else { continue };
        if used >= TOTAL_MAX {
            p.push_str("(나머지 파일의 Diff 는 길어서 뺐다)\n");
            break;
        }
        let part = cut(d, FILE_MAX.min(TOTAL_MAX - used));
        p.push_str(&format!("--- {}\n{}", f.path, part));
        if part.len() < d.len() {
            p.push_str("(이 파일의 Diff 는 길어서 앞부분만 넣었다)\n");
        }
        used += part.len();
    }
    p
}

/* ---------- 실행 ---------- */

/// 설정의 명령을 cmd /c 로 부르고 프롬프트를 stdin 으로 넘긴다. 답의 첫 줄을 다듬어 돌려준다.
pub fn run(cmd: &str, prompt: &str) -> Result<String, String> {
    let cmd = cmd.trim();
    if cmd.is_empty() {
        return Err("「설정 › 경로 › Commit message 생성 명령」이 비어 있습니다.".into());
    }
    let mut c = Command::new("cmd");
    // raw_arg : 적힌 그대로 cmd 에 넘긴다. arg 로 넘기면 경로에 든 따옴표가 \" 로 바뀌어 cmd 가 못 읽는다
    c.arg("/c").raw_arg(cmd);
    let out = svncli::run(c, Some(prompt), GEN_LIMIT).map_err(|e| format!("Commit message 생성 명령이 실패했습니다 ({cmd}): {e}"))?;
    if !out.ok {
        let why: String = out.stderr.chars().take(400).collect();
        return Err(format!("Commit message 생성 명령이 실패했습니다 ({cmd}){}", if why.is_empty() { String::new() } else { format!(": {why}") }));
    }
    let msg = clean(&decode(&out.stdout));
    if msg.is_empty() {
        return Err(format!("Commit message 생성 명령이 빈 답을 냈습니다 ({cmd})."));
    }
    Ok(msg)
}

/// 비지 않은 첫 줄에서 감싼 따옴표 · 백틱 · 「제목:」 같은 머리를 벗긴다
pub fn clean(answer: &str) -> String {
    let line = answer.lines().map(str::trim).find(|l| !l.is_empty() && !l.starts_with("```")).unwrap_or("");
    let mut s = line.trim_start_matches(['#', '*', '-', ' ']).trim();
    for head in ["Commit message:", "commit message:", "제목:", "제목 :", "Subject:", "subject:"] {
        if let Some(rest) = s.strip_prefix(head) {
            s = rest;
        }
    }
    // 「**제목:** 내용」 처럼 머리 뒤에 남은 굵게 표시
    s = s.trim_matches(['*', ' ']);
    let pairs = [('"', '"'), ('\'', '\''), ('`', '`'), ('「', '」'), ('“', '”')];
    for (a, b) in pairs {
        if s.chars().count() >= 2 && s.starts_with(a) && s.ends_with(b) {
            s = &s[a.len_utf8()..s.len() - b.len_utf8()];
        }
    }
    s.trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::svn::DiffLine;

    fn rule() -> Rule {
        Rule {
            text: "이슈 키 뒤에 한 줄 평서문".into(),
            example: "FDR-106 접수 현황을 기간으로 조회한다".into(),
            issue_pattern: r"[A-Z][A-Z0-9]+-\d+".into(),
            subject_pattern: r"^[A-Z]+-\d+ .+다$".into(),
            subject_max: 20,
        }
    }

    #[test]
    fn 규칙을_검사한다() {
        let r = rule();
        assert!(check(&r, "FDR-1 목록을 정렬한다").is_empty());
        assert_eq!(check(&r, "목록을 정렬한다").len(), 1);
        assert_eq!(check(&r, "  ").len(), 1);
        // 한글 20자는 통과 · 21자는 걸린다 (바이트로 세면 60바이트라 걸렸을 것)
        assert!(check(&r, &format!("FDR-1 {}다", "가".repeat(13))).is_empty());
        assert_eq!(check(&r, &format!("FDR-1 {}다", "가".repeat(14))).len(), 1);
        // 규칙이 없으면 빈 메시지만 걸린다
        assert!(check(&Rule::default(), "아무 말").is_empty());
        assert!(validate(&Rule { subject_pattern: "(".into(), ..Rule::default() }).is_err());
    }

    #[test]
    fn 이슈_번호를_찾는다() {
        assert_eq!(issue_of(r"[A-Z][A-Z0-9]+-\d+", "feature/FDR-112-order").as_deref(), Some("FDR-112"));
        assert_eq!(issue_of(r"[A-Z][A-Z0-9]+-\d+", "main"), None);
        assert_eq!(issue_of("", "FDR-1"), None);
    }

    #[test]
    fn 답의_머리와_따옴표를_벗긴다() {
        assert_eq!(clean("\n\"FDR-1 목록을 정렬한다\"\n\n설명..."), "FDR-1 목록을 정렬한다");
        assert_eq!(clean("```\n제목: `접수 화면을 고친다`\n```"), "접수 화면을 고친다");
        assert_eq!(clean("**Commit message:** 「목록 정렬」"), "목록 정렬");
        assert_eq!(clean(""), "");
    }

    #[test]
    fn 프롬프트에_규칙과_diff_를_넣고_길면_자른다() {
        let d = Diff {
            path: "a.java".into(),
            note: None,
            added: 1,
            removed: 1,
            lines: vec![
                DiffLine { t: "hunk", o: None, n: None, s: "@@ -1,1 +1,1 @@".into() },
                DiffLine { t: "del", o: Some(1), n: None, s: "old".into() },
                DiffLine { t: "add", o: None, n: Some(1), s: "새 줄".into() },
            ],
        };
        let t = diff_text(&d).unwrap();
        assert_eq!(t, "@@ -1,1 +1,1 @@\n-old\n+새 줄\n");
        let big = "가".repeat(FILE_MAX); // 3바이트 글자라 FILE_MAX 의 세 배
        let files = vec![
            FileDiff { path: "a.java".into(), label: "Modified".into(), diff: Some(t) },
            FileDiff { path: "b.png".into(), label: "Added".into(), diff: None },
            FileDiff { path: "c.jsp".into(), label: "Modified".into(), diff: Some(big) },
        ];
        let p = build_prompt(&rule(), Some("FDR-112"), "feature/FDR-112", &files);
        assert!(p.contains("[팀 규칙] 이슈 키 뒤에 한 줄 평서문"));
        assert!(p.contains("[이슈 번호 (규칙이 이슈 번호를 요구하면 이것을 쓴다)] FDR-112"));
        assert!(p.contains("- Added b.png"));
        assert!(p.contains("+새 줄"));
        assert!(p.contains("앞부분만"));
        assert!(p.len() < FILE_MAX * 2 + 2048);
        // 빈 칸은 줄을 만들지 않는다
        assert!(!build_prompt(&Rule::default(), None, "", &[]).contains("[예시]"));
    }

    #[test]
    fn 명령에_stdin_으로_넘기고_첫_줄을_받는다() {
        // findstr 는 stdin 을 그대로 내보낸다 — 프롬프트에 든 & · % · 따옴표가 cmd 에서 해석되지 않아야 한다
        let got = run("findstr \"^\"", "\"FDR-1 a & b %PATH% 고친다\"\n둘째 줄\n").unwrap();
        assert_eq!(got, "FDR-1 a & b %PATH% 고친다");
        assert!(run("", "x").is_err());
        assert!(run("exit /b 3", "x").is_err());
    }
}
