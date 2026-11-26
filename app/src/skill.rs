// 에이전트 스킬 — 코딩 에이전트가 제어 창구(ctl.rs)로 톰캣을 다루게 하는 안내서(SKILL.md)와 부르는 명령줄(tomcatrun.mjs)을 깐다.
//
// 본문의 정본은 저장소의 skill/tomcatrun/ 이다. 실행 파일에 넣어 두고, 설치할 때 SKILL.md 의 {{MJS}} 를 깐 자리의 tomcatrun.mjs 경로로,
// {{VERSION}} 을 이 판으로 바꾼다. 설정 › 에이전트 스킬이 이 PC 에 깔린 에이전트를 찾아 보이고 고른 곳마다 설치 · 제거한다.
//
// 2026.11.26 : filedesk(_tools/filedesk/crates/fd-core/src/skill.rs)와 같은 표 · 같은 규칙으로 만들었다. 에이전트마다 다른 것은 자리뿐이고,
// 본문은 공용 스킬 꼴(<자리>/tomcatrun/SKILL.md · 머리말 name · description)이라 어느 에이전트나 그대로 읽는다.
// 전에는 README 대로 ~/.claude/skills 에 손으로 복사했다 — 그렇게 깐 것도 name 이 tomcatrun 이면 우리 것으로 보고 켤 때 이 판으로 다시 쓴다.

use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::Command;

use serde::Serialize;

const SKILL: &str = include_str!("../../skill/tomcatrun/SKILL.md");
const MJS: &str = include_str!("../../skill/tomcatrun/tomcatrun.mjs");
const NAME_LINE: &str = "name: tomcatrun";

/// 스킬을 읽는 에이전트 하나. 경로는 모두 사용자 폴더 기준.
struct Agent {
    id: &'static str,
    name: &'static str,
    /// 이 폴더가 있으면 깔린 것으로 본다 (에이전트가 처음 켤 때 만든다).
    marker: &'static str,
    /// 스킬 폴더들의 자리.
    dir: &'static str,
}

/// 2026.11.26 : filedesk 의 표를 그대로 옮겼다 (그쪽 2026.11.13 확인 — claude · codex · copilot 은 실기, 나머지는 각 도구 문서대로).
/// codex 는 공용 자리 ~/.agents/skills 를 읽는다 — 옛 자리 ~/.codex/skills 에도 쓰면 같은 스킬이 두 번 뜬다.
const AGENTS: &[Agent] = &[
    Agent { id: "claude",   name: "Claude Code",    marker: ".claude",          dir: ".claude/skills" },
    Agent { id: "codex",    name: "Codex",          marker: ".codex",           dir: ".agents/skills" },
    Agent { id: "copilot",  name: "GitHub Copilot", marker: ".copilot",         dir: ".copilot/skills" },
    Agent { id: "gemini",   name: "Gemini CLI",     marker: ".gemini",          dir: ".gemini/skills" },
    Agent { id: "cursor",   name: "Cursor",         marker: ".cursor",          dir: ".cursor/skills" },
    Agent { id: "opencode", name: "OpenCode",       marker: ".config/opencode", dir: ".config/opencode/skills" },
];

/// 에이전트 하나의 설치 상태 — 설정 화면이 그대로 그린다.
#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Target {
    pub id: String,
    pub name: String,
    /// 설치 폴더 (`…/tomcatrun`).
    pub dir: PathBuf,
    /// 이 PC 에 깔려 있는가.
    pub detected: bool,
    /// 우리 스킬이 깔려 있는가 (머리말 name 이 tomcatrun).
    pub installed: bool,
    /// 깔린 스킬의 판. 손으로 복사한 옛 본문처럼 판이 적혀 있지 않으면 None.
    pub version: Option<String>,
}

/// 설정 › 에이전트 스킬이 받는 것.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Status {
    agents: Vec<Target>,
    current: &'static str,
    /// tomcatrun.mjs 를 돌릴 Node 가 PATH 에 있는가 — 없으면 깔아도 에이전트가 부르지 못한다.
    has_node: bool,
}

fn home() -> Option<PathBuf> {
    std::env::var_os("USERPROFILE").map(PathBuf::from)
}

fn target(home: &Path, a: &Agent) -> Target {
    // 빗금으로 적은 자리를 마디로 이어 Windows 꼴(역빗금)로만 만든다 — 섞인 경로는 화면에서 어색하다.
    let at = |rel: &str| rel.split('/').fold(home.to_path_buf(), |p, c| p.join(c));
    let dir = at(a.dir).join("tomcatrun");
    let body = std::fs::read_to_string(dir.join("SKILL.md")).ok();
    Target {
        id: a.id.into(),
        name: a.name.into(),
        detected: at(a.marker).is_dir(),
        installed: body.as_deref().is_some_and(ours),
        version: body.as_deref().and_then(version_of),
        dir,
    }
}

/// 모든 에이전트의 상태 (감지 못 한 것도 — 화면이 이름만 한 줄로 보인다).
fn targets() -> Vec<Target> {
    let Some(h) = home() else { return Vec::new() };
    AGENTS.iter().map(|a| target(&h, a)).collect()
}

fn find(id: &str) -> Result<Target, String> {
    targets().into_iter().find(|t| t.id == id).ok_or_else(|| format!("모르는 에이전트: {id}"))
}

fn ours(body: &str) -> bool {
    body.lines().any(|l| l.trim() == NAME_LINE)
}

/// 본문에 적힌 판 — 「tomcatrun 」 바로 뒤에 숫자가 오는 첫 자리 (`tomcatrun 실행기(0.32.0` 처럼 사이에 글이 낀 곳은 건너뛴다).
fn version_of(body: &str) -> Option<String> {
    body.match_indices("tomcatrun ").find_map(|(i, m)| {
        let v: String = body[i + m.len()..].chars().take_while(|c| c.is_ascii_digit() || *c == '.').collect();
        (v.contains('.')).then_some(v)
    })
}

fn mjs_path(dir: &Path) -> String {
    dir.join("tomcatrun.mjs").to_string_lossy().replace('\\', "/")
}

/// 설치할 SKILL.md 본문.
fn render(dir: &Path) -> String {
    SKILL.replace("{{MJS}}", &mjs_path(dir)).replace("{{VERSION}}", env!("CARGO_PKG_VERSION"))
}

fn has_node() -> bool {
    Command::new("node").arg("--version").creation_flags(0x0800_0000).output().is_ok_and(|o| o.status.success())
}

pub fn status() -> Status {
    Status { agents: targets(), current: env!("CARGO_PKG_VERSION"), has_node: has_node() }
}

/// 깐다 — 같은 이름의 남의 스킬이 있으면 덮지 않는다.
fn install_at(dir: &Path) -> Result<(), String> {
    let skill = dir.join("SKILL.md");
    if let Ok(s) = std::fs::read_to_string(&skill) {
        if !ours(&s) {
            return Err(format!("{}: 이름이 같은 다른 스킬이 있어 덮지 않았습니다", skill.display()));
        }
    }
    std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    std::fs::write(dir.join("tomcatrun.mjs"), MJS).map_err(|e| format!("{}: {e}", dir.display()))?;
    std::fs::write(&skill, render(dir)).map_err(|e| format!("{}: {e}", skill.display()))
}

/// 뺀다 — 우리가 쓴 두 파일만 지우고, 폴더가 비면 폴더도. 남의 스킬이면 손대지 않는다.
fn uninstall_at(dir: &Path) -> Result<(), String> {
    let skill = dir.join("SKILL.md");
    let Ok(s) = std::fs::read_to_string(&skill) else { return Ok(()) };
    if !ours(&s) {
        return Err(format!("{}: tomcatrun 스킬이 아니라 지우지 않았습니다", skill.display()));
    }
    for f in ["SKILL.md", "tomcatrun.mjs"] {
        let p = dir.join(f);
        if p.exists() {
            std::fs::remove_file(&p).map_err(|e| format!("{}: {e}", p.display()))?;
        }
    }
    let _ = std::fs::remove_dir(dir); // 사람이 더 넣은 파일이 있으면 비지 않아 실패한다 — 그대로 둔다
    Ok(())
}

/// 켤 때 다시 쓸 것인가 — 판이 다르거나, 본문에 이 자리의 mjs 경로가 없거나, mjs 가 이 판과 다를 때만.
/// filedesk 2026.11.11 리뷰와 같은 까닭 : 본문이 조금만 달라도 덮으면 사람이 고친 스킬이 켤 때마다 사라진다.
fn needs_refresh(dir: &Path) -> bool {
    let Ok(s) = std::fs::read_to_string(dir.join("SKILL.md")) else { return false };
    if !ours(&s) {
        return false;
    }
    let mjs_same = std::fs::read_to_string(dir.join("tomcatrun.mjs")).is_ok_and(|m| m.replace("\r\n", "\n") == MJS.replace("\r\n", "\n"));
    version_of(&s).as_deref() != Some(env!("CARGO_PKG_VERSION")) || !s.contains(&mjs_path(dir)) || !mjs_same
}

pub fn install(agent: &str) -> Result<Status, String> {
    install_at(&find(agent)?.dir)?;
    Ok(status())
}

pub fn uninstall(agent: &str) -> Result<Status, String> {
    uninstall_at(&find(agent)?.dir)?;
    Ok(status())
}

/// 켤 때 — 이미 깐 곳만 이 판으로 맞춘다 (깔지 않은 에이전트에 새로 깔지 않는다). 돌려주는 값은 다시 쓴 에이전트 이름.
pub fn refresh() -> Vec<String> {
    targets()
        .into_iter()
        .filter(|t| needs_refresh(&t.dir) && install_at(&t.dir).is_ok())
        .map(|t| t.name)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(tag: &str) -> PathBuf {
        std::env::temp_dir().join(format!("tomcatrun-skill-{tag}-{}", std::process::id()))
    }

    #[test]
    fn 깔면_경로와_판을_넣고_같은_판이면_사람이_고친_것을_둔다() {
        let dir = tmp("a").join("tomcatrun");
        install_at(&dir).unwrap();
        let s = std::fs::read_to_string(dir.join("SKILL.md")).unwrap();
        assert!(!s.contains("{{"));
        assert!(s.contains(&mjs_path(&dir)));
        assert_eq!(version_of(&s).as_deref(), Some(env!("CARGO_PKG_VERSION")));
        assert!(dir.join("tomcatrun.mjs").exists());
        assert!(!needs_refresh(&dir));
        std::fs::write(dir.join("SKILL.md"), s.clone() + "\n내가 더한 줄\n").unwrap();
        assert!(!needs_refresh(&dir));
        // 판이 적히지 않은 옛 복사본은 다시 쓴다
        std::fs::write(dir.join("SKILL.md"), "---\nname: tomcatrun\n---\ntomcatrun 실행기(0.32.0 이상)\n").unwrap();
        assert!(needs_refresh(&dir));
        uninstall_at(&dir).unwrap();
        assert!(!dir.exists());
        let _ = std::fs::remove_dir_all(tmp("a"));
    }

    #[test]
    fn 남의_스킬은_덮지도_지우지도_않는다() {
        let dir = tmp("b").join("tomcatrun");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("SKILL.md"), "---\nname: other\n---\n").unwrap();
        assert!(install_at(&dir).is_err());
        assert!(uninstall_at(&dir).is_err());
        assert!(!needs_refresh(&dir));
        let _ = std::fs::remove_dir_all(tmp("b"));
    }

    #[test]
    fn 표시_폴더로_감지하고_codex_는_공용_자리에_깐다() {
        let h = tmp("home");
        std::fs::create_dir_all(h.join(".codex")).unwrap();
        let codex = target(&h, AGENTS.iter().find(|a| a.id == "codex").unwrap());
        assert!(codex.detected && !codex.installed);
        assert!(codex.dir.ends_with(Path::new(".agents").join("skills").join("tomcatrun")));
        assert!(!target(&h, &AGENTS[0]).detected);
        let _ = std::fs::remove_dir_all(&h);
    }
}
