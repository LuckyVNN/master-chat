//! Native menus: an account's right-click menu, Do Not Disturb, and the tray.
//! Menu item ids say what to do: "acct:<action>:<account id>", "dnd:<choice>",
//! "tray:<action>".

use crate::{call_shell, now_secs, AppState};
use tauri::menu::{CheckMenuItem, IsMenuItem, Menu, MenuItem, PredefinedMenuItem, Submenu};
use tauri::{AppHandle, Manager, Wry};

pub const TRAY_ID: &str = "main";

fn err(e: tauri::Error) -> String {
    e.to_string()
}

fn item(app: &AppHandle, id: String, text: &str, enabled: bool) -> Result<MenuItem<Wry>, String> {
    MenuItem::with_id(app, id, text, enabled, None::<&str>).map_err(err)
}

fn check(app: &AppHandle, id: String, text: &str, checked: bool) -> Result<CheckMenuItem<Wry>, String> {
    CheckMenuItem::with_id(app, id, text, true, checked, None::<&str>).map_err(err)
}

fn separator(app: &AppHandle) -> Result<PredefinedMenuItem<Wry>, String> {
    PredefinedMenuItem::separator(app).map_err(err)
}

pub fn account_menu(app: &AppHandle, id: &str) -> Result<Menu<Wry>, String> {
    let state = app.state::<AppState>();
    let (account, index, count) = {
        let inner = state.inner.lock().unwrap();
        let index = inner.accounts.iter().position(|a| a.id == id).ok_or("unknown account")?;
        (inner.accounts[index].clone(), index, inner.accounts.len())
    };
    let a = |action: &str| format!("acct:{action}:{id}");
    let zoom = Submenu::with_items(
        app,
        format!("Zoom ({}%)", (account.zoom * 100.0).round()),
        true,
        &[
            &item(app, a("zoom_in"), "Zoom in", account.zoom < 3.0)?,
            &item(app, a("zoom_out"), "Zoom out", account.zoom > 0.5)?,
            &item(app, a("zoom_reset"), "Actual size", account.zoom != 1.0)?,
        ],
    )
    .map_err(err)?;
    let reload = item(app, a("reload"), "Reload", true)?;
    let edit = item(app, a("edit"), "Edit name and color…", true)?;
    let notify = check(app, a("notify"), "Notifications", account.notify)?;
    let typing = check(app, a("typing"), "Hide that I'm typing", account.block_typing)?;
    let up = item(app, a("up"), "Move up", index > 0)?;
    let down = item(app, a("down"), "Move down", index + 1 < count)?;
    let remove = item(app, a("remove"), "Remove…", true)?;
    let (sep1, sep2, sep3) = (separator(app)?, separator(app)?, separator(app)?);
    let mut items: Vec<&dyn IsMenuItem<Wry>> = vec![&reload, &edit, &sep1, &notify];
    if account.service == "messenger" {
        items.push(&typing);
    }
    items.extend([&zoom as &dyn IsMenuItem<Wry>, &sep2, &up, &down, &sep3, &remove]);
    Menu::with_items(app, &items).map_err(err)
}

fn dnd_items(app: &AppHandle) -> Result<Vec<CheckMenuItem<Wry>>, String> {
    let state = app.state::<AppState>();
    let (active, forever) = {
        let s = state.settings.lock().unwrap();
        (s.dnd_active(), s.dnd_until == Some(u64::MAX))
    };
    Ok(vec![
        check(app, "dnd:off".into(), "Off", !active)?,
        check(app, "dnd:30".into(), "For 30 minutes", false)?,
        check(app, "dnd:60".into(), "For 1 hour", false)?,
        check(app, "dnd:480".into(), "For 8 hours", false)?,
        check(app, "dnd:forever".into(), "Until I turn it off", active && forever)?,
    ])
}

pub fn dnd_menu(app: &AppHandle) -> Result<Menu<Wry>, String> {
    let items = dnd_items(app)?;
    let refs: Vec<&dyn IsMenuItem<Wry>> = items.iter().map(|i| i as &dyn IsMenuItem<Wry>).collect();
    Menu::with_items(app, &refs).map_err(err)
}

pub fn tray_menu(app: &AppHandle) -> Result<Menu<Wry>, String> {
    let state = app.state::<AppState>();
    let lock_enabled = state.settings.lock().unwrap().lock.enabled;
    let dnd = dnd_items(app)?;
    let dnd_refs: Vec<&dyn IsMenuItem<Wry>> = dnd.iter().map(|i| i as &dyn IsMenuItem<Wry>).collect();
    let dnd = Submenu::with_items(app, "Do Not Disturb", true, &dnd_refs).map_err(err)?;
    Menu::with_items(
        app,
        &[
            &item(app, "tray:show".into(), "Open Master Chat", true)?,
            &separator(app)?,
            &dnd,
            &item(app, "tray:lock".into(), "Lock now", lock_enabled)?,
            &separator(app)?,
            &item(app, "tray:quit".into(), "Quit", true)?,
        ],
    )
    .map_err(err)
}

/// Rebuilds the tray's menu after a setting it shows changed.
pub fn refresh_tray(app: &AppHandle) {
    if let (Some(tray), Ok(menu)) = (app.tray_by_id(TRAY_ID), tray_menu(app)) {
        let _ = tray.set_menu(Some(menu));
    }
}

pub fn on_menu_event(app: &AppHandle, menu_id: &str) {
    let mut parts = menu_id.splitn(3, ':');
    match (parts.next(), parts.next(), parts.next()) {
        (Some("acct"), Some(action), Some(id)) => on_account_action(app, action, id),
        (Some("dnd"), Some(choice), None) => {
            let until = match choice {
                "off" => None,
                "forever" => Some(u64::MAX),
                minutes => minutes.parse::<u64>().ok().map(|m| now_secs() + m * 60),
            };
            crate::set_dnd_until(app, until);
        }
        (Some("tray"), Some("show"), None) => crate::show_window(app),
        (Some("tray"), Some("lock"), None) => crate::lock_app(app),
        (Some("tray"), Some("quit"), None) => app.exit(0),
        _ => {}
    }
}

fn on_account_action(app: &AppHandle, action: &str, id: &str) {
    let state = app.state::<AppState>();
    if state.inner.lock().unwrap().locked {
        return;
    }
    let id_js = serde_json::to_string(id).unwrap_or_default();
    match action {
        "reload" => {
            if let Some(w) = app.get_webview(&crate::webview_label(id)) {
                state.inner.lock().unwrap().loaded_at.insert(id.to_string(), crate::now_millis());
                let _ = w.reload();
            }
        }
        // Editing and removing ask in the shell, which then calls the commands.
        "edit" => call_shell(app, format!("window.masterChat.edit({id_js})")),
        "remove" => call_shell(app, format!("window.masterChat.remove({id_js})")),
        "notify" | "typing" | "zoom_in" | "zoom_out" | "zoom_reset" | "up" | "down" => {
            let _ = crate::change_account(app, id, |accounts, i| {
                let count = accounts.len();
                let zoom = accounts[i].zoom;
                match action {
                    "notify" => accounts[i].notify = !accounts[i].notify,
                    "typing" => accounts[i].block_typing = !accounts[i].block_typing,
                    "zoom_in" => accounts[i].zoom = crate::accounts::clamp_zoom(zoom + 0.1),
                    "zoom_out" => accounts[i].zoom = crate::accounts::clamp_zoom(zoom - 0.1),
                    "zoom_reset" => accounts[i].zoom = 1.0,
                    "up" if i > 0 => accounts.swap(i, i - 1),
                    "down" if i + 1 < count => accounts.swap(i, i + 1),
                    _ => {}
                }
            });
        }
        _ => {}
    }
}
