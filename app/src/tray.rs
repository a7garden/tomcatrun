// 시스템 트레이 — 톰캣이 켜진 채 창을 닫으면 실행기는 끝나지 않고 트레이 아이콘으로 남는다 (2026.11.13~).
// 실행기가 끝나면 작업 개체가 톰캣까지 함께 끄므로, 톰캣을 살려 두려면 실행기 프로세스가 살아 있어야 한다.
//
//   아이콘 왼쪽 클릭 · 「창 열기」   창을 다시 보인다
//   「종료」                         켜진 톰캣이 있으면 창을 띄워 묻고(close-ask), 없으면 바로 끝낸다
//   풍선 도움말                       켜진 서버 이름 — 창을 숨겨 두어도 무엇이 돌고 있는지 보인다
use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Manager};

const ID: &str = "main";

pub fn build(app: &AppHandle) -> tauri::Result<()> {
    let open = MenuItem::with_id(app, "open", "창 열기", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "종료", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&open, &PredefinedMenuItem::separator(app)?, &quit])?;
    let mut b = TrayIconBuilder::with_id(ID)
        .tooltip(tooltip(&[]))
        .menu(&menu)
        .show_menu_on_left_click(false) // 왼쪽은 창 열기, 메뉴는 오른쪽 클릭
        .on_menu_event(|app, e| match e.id().as_ref() {
            "open" => show(app),
            "quit" => crate::quit_from_tray(app),
            _ => {}
        })
        .on_tray_icon_event(|tray, e| {
            if let TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, .. } = e {
                show(tray.app_handle());
            }
        });
    if let Some(icon) = app.default_window_icon() {
        b = b.icon(icon.clone());
    }
    b.build(app)?;
    Ok(())
}

pub fn show(app: &AppHandle) {
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.show();
        let _ = w.unminimize();
        let _ = w.set_focus();
    }
}

pub fn hide(app: &AppHandle) {
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.hide();
    }
}

// 트레이 아이콘이 떠 있는가 — 없으면(만들다 실패) 창을 숨기면 되돌아올 길이 없다
pub fn exists(app: &AppHandle) -> bool {
    app.tray_by_id(ID).is_some()
}

pub fn set_running(app: &AppHandle, running: &[String]) {
    if let Some(t) = app.tray_by_id(ID) {
        let _ = t.set_tooltip(Some(tooltip(running)));
    }
}

fn tooltip(running: &[String]) -> String {
    if running.is_empty() {
        "tomcatrun · 켜진 톰캣 없음".into()
    } else {
        format!("tomcatrun · 실행 중: {}", running.join(", "))
    }
}
