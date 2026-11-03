mod accounts;
mod badge;
mod google_domains;
mod links;
mod lock;
mod menus;
mod services;
mod settings;
mod system;

use accounts::Account;
use serde::Serialize;
use settings::{Settings, SettingsUpdate, SettingsView};
use std::{
    collections::{HashMap, VecDeque},
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use tauri::{
    image::Image,
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    webview::{NewWindowResponse, WebviewBuilder},
    window::WindowBuilder,
    AppHandle, LogicalPosition, LogicalSize, Manager, State, Theme, Url, Webview, WebviewUrl, Window, WindowEvent,
};
use tauri_plugin_global_shortcut::{GlobalShortcutExt, ShortcutState};
use tauri_plugin_updater::UpdaterExt;

/// Width of the local sidebar; service webviews fill the rest of the window.
const SIDEBAR: f64 = 68.0;
/// Label of the local UI webview, the only one allowed to manage the app.
const SHELL: &str = "shell";
/// Hides the window (and locks the app, if the lock is on) from anywhere.
const PANIC_SHORTCUT: &str = "ctrl+alt+h";
/// A service may show at most this many notifications per 10 seconds.
const NOTIFY_BURST: usize = 5;

pub(crate) struct Inner {
    accounts: Vec<Account>,
    active: Option<String>,
    /// True while a panel in the shell (add account, settings) needs the whole window.
    overlay: bool,
    unread: HashMap<String, u32>,
    locked: bool,
    failed_unlocks: u32,
    /// No PIN is checked before this time (seconds), after wrong guesses.
    unlock_retry_at: u64,
    focused: bool,
    notify_log: HashMap<String, VecDeque<u128>>,
    /// Which account webviews were last asked to use little memory.
    low_memory: HashMap<String, bool>,
}

pub(crate) struct AppState {
    inner: Mutex<Inner>,
    settings: Mutex<Settings>,
    accounts_file: PathBuf,
    settings_file: PathBuf,
    profiles: PathBuf,
}

#[derive(Serialize)]
struct AccountView {
    id: String,
    service: String,
    label: String,
    color: Option<String>,
    unread: u32,
    notify: bool,
}

#[derive(Serialize)]
struct Snapshot {
    accounts: Vec<AccountView>,
    active: Option<String>,
    locked: bool,
    settings: SettingsView,
    version: String,
}

pub(crate) fn now_secs() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

fn now_millis() -> u128 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_millis())
}

pub(crate) fn webview_label(id: &str) -> String {
    format!("acc-{id}")
}

/// Defense in depth on top of the capability ACL: management commands only
/// answer the local shell, and only while the app is unlocked.
fn guard(webview: &Webview, state: &AppState) -> Result<(), String> {
    guard_shell(webview)?;
    if state.inner.lock().unwrap().locked {
        return Err("locked".into());
    }
    Ok(())
}

fn guard_shell(webview: &Webview) -> Result<(), String> {
    if webview.label() == SHELL {
        Ok(())
    } else {
        Err("forbidden".into())
    }
}

/// Links that leave a service go to the default browser, cleaned of redirects
/// and tracking; other schemes are dropped.
fn open_external(url: &Url) {
    if matches!(url.scheme(), "http" | "https" | "mailto") {
        let url = links::clean(url);
        let _ = tauri_plugin_opener::open_url(url.as_str(), None::<&str>);
    }
}

fn snapshot(app: &AppHandle) -> Snapshot {
    let state = app.state::<AppState>();
    let settings = SettingsView::from(&*state.settings.lock().unwrap());
    let inner = state.inner.lock().unwrap();
    // The lock screen shows nothing about the accounts.
    let accounts = if inner.locked {
        Vec::new()
    } else {
        inner
            .accounts
            .iter()
            .map(|a| AccountView {
                id: a.id.clone(),
                service: a.service.clone(),
                label: a.label.clone(),
                color: a.color.clone(),
                unread: inner.unread.get(&a.id).copied().unwrap_or(0),
                notify: a.notify,
            })
            .collect()
    };
    Snapshot {
        accounts,
        active: if inner.locked { None } else { inner.active.clone() },
        locked: inner.locked,
        settings,
        version: app.package_info().version.to_string(),
    }
}

pub(crate) fn call_shell(app: &AppHandle, js: String) {
    if let Some(shell) = app.get_webview(SHELL) {
        let _ = shell.eval(js);
    }
}

fn refresh_shell(app: &AppHandle) {
    call_shell(app, "window.masterChat && window.masterChat.refresh()".into());
}

/// Places the shell under everything and shows only the active account's webview.
fn layout(app: &AppHandle) {
    let Some(window) = app.get_window("main") else { return };
    let (Ok(size), Ok(scale)) = (window.inner_size(), window.scale_factor()) else { return };
    let size = size.to_logical::<f64>(scale);

    if let Some(shell) = app.get_webview(SHELL) {
        let _ = shell.set_position(LogicalPosition::new(0.0, 0.0));
        let _ = shell.set_size(size);
    }

    // Collect first so no webview call runs while the lock is held. The memory
    // level is only sent when it changes, not on every resize.
    let visible: Vec<(String, bool, bool)> = {
        let state = app.state::<AppState>();
        let mut inner = state.inner.lock().unwrap();
        let hidden = inner.overlay || inner.locked;
        let shown: Vec<(String, bool)> = inner
            .accounts
            .iter()
            .map(|a| (a.id.clone(), !hidden && inner.active.as_deref() == Some(&a.id)))
            .collect();
        shown
            .into_iter()
            .map(|(id, show)| {
                let changed = inner.low_memory.insert(id.clone(), !show) != Some(!show);
                (webview_label(&id), show, changed)
            })
            .collect()
    };
    for (label, show, memory_changed) in visible {
        let Some(webview) = app.get_webview(&label) else { continue };
        if memory_changed {
            system::set_low_memory(&webview, !show);
        }
        if show {
            let _ = webview.set_position(LogicalPosition::new(SIDEBAR, 0.0));
            let _ = webview.set_size(LogicalSize::new((size.width - SIDEBAR).max(1.0), size.height));
            let _ = webview.show();
        } else {
            let _ = webview.hide();
        }
    }
}

/// The window title, tray tooltip and taskbar badge follow the total unread count.
fn show_total_unread(app: &AppHandle) {
    let total: u32 = {
        let state = app.state::<AppState>();
        let inner = state.inner.lock().unwrap();
        inner.unread.values().sum()
    };
    let title = if total > 0 { format!("Master Chat ({total})") } else { "Master Chat".into() };
    if let Some(window) = app.get_window("main") {
        let _ = window.set_title(&title);
        let badge = (total > 0).then(|| {
            let (rgba, w, h) = badge::render(total);
            Image::new_owned(rgba, w, h)
        });
        let _ = window.set_overlay_icon(badge);
    }
    if let Some(tray) = app.tray_by_id(menus::TRAY_ID) {
        let tip = if total > 0 { format!("Master Chat: {total} unread") } else { "Master Chat".into() };
        let _ = tray.set_tooltip(Some(tip));
    }
}

fn set_unread(app: &AppHandle, id: &str, count: u32) {
    {
        let state = app.state::<AppState>();
        let mut inner = state.inner.lock().unwrap();
        if inner.unread.get(id) == Some(&count) {
            return;
        }
        inner.unread.insert(id.to_string(), count);
    }
    show_total_unread(app);
    let id = serde_json::to_string(id).unwrap_or_default();
    call_shell(app, format!("window.masterChat && window.masterChat.unread({id}, {count})"));
}

pub(crate) fn show_window(app: &AppHandle) {
    if let Some(window) = app.get_window("main") {
        let _ = window.show();
        let _ = window.unminimize();
        let _ = window.set_focus();
    }
}

fn apply_theme(app: &AppHandle, theme: &str) {
    let theme = match theme {
        "light" => Some(Theme::Light),
        "dark" => Some(Theme::Dark),
        _ => None,
    };
    if let Some(window) = app.get_window("main") {
        let _ = window.set_theme(theme);
    }
}

fn select(app: &AppHandle, id: &str) {
    {
        let state = app.state::<AppState>();
        let mut inner = state.inner.lock().unwrap();
        if inner.locked || !inner.accounts.iter().any(|a| a.id == id) {
            return;
        }
        inner.active = Some(id.to_string());
        inner.overlay = false;
    }
    layout(app);
    if let Some(w) = app.get_webview(&webview_label(id)) {
        let _ = w.set_focus();
    }
}

pub(crate) fn lock_app(app: &AppHandle) {
    let state = app.state::<AppState>();
    if !state.settings.lock().unwrap().lock.enabled {
        return;
    }
    state.inner.lock().unwrap().locked = true;
    layout(app);
    refresh_shell(app);
}

pub(crate) fn set_dnd_until(app: &AppHandle, until: Option<u64>) {
    let state = app.state::<AppState>();
    {
        let mut settings = state.settings.lock().unwrap();
        settings.dnd_until = until;
        let _ = settings::save(&state.settings_file, &settings);
    }
    menus::refresh_tray(app);
    refresh_shell(app);
}

/// Changes an account through `change` (given all accounts and its index),
/// saves, and applies what changed to its webview.
pub(crate) fn change_account(
    app: &AppHandle,
    id: &str,
    change: impl FnOnce(&mut Vec<Account>, usize),
) -> Result<(), String> {
    let state = app.state::<AppState>();
    let account = {
        let mut inner = state.inner.lock().unwrap();
        let index = inner.accounts.iter().position(|a| a.id == id).ok_or("unknown account")?;
        change(&mut inner.accounts, index);
        accounts::save(&state.accounts_file, &inner.accounts)?;
        inner.accounts.iter().find(|a| a.id == id).cloned().ok_or("unknown account")?
    };
    if let Some(w) = app.get_webview(&webview_label(id)) {
        let _ = w.set_zoom(account.zoom);
        let _ = w.eval(format!("(window.__mcSettings ||= {{}}).blockTyping = {}", account.block_typing));
    }
    refresh_shell(app);
    Ok(())
}

/// The scripts a service's page gets at document start: the notifications
/// shim everywhere, the account's options, and the service's own restyling.
fn page_script(service: &services::Service, account: &Account) -> String {
    let options = serde_json::json!({ "blockTyping": account.block_typing });
    let mut script = format!("window.__mcSettings = {options};\n{}", include_str!("inject/notify.js"));
    if let Some(inject) = service.inject {
        script.push('\n');
        script.push_str(inject);
    }
    script
}

/// Each account gets its own browser profile folder, so cookies and logins never mix.
/// The webview starts outside the window and unfocused, so one loading in the
/// background never flashes over the lock screen or takes the keyboard; `layout`
/// then shows it if it's the active account.
fn create_account_webview(app: &AppHandle, window: &Window, account: &Account, profiles: &Path) -> Result<(), String> {
    let service = services::find(&account.service).ok_or("unknown service")?;
    let url = Url::parse(service.url).map_err(|e| e.to_string())?;
    let id = account.id.clone();
    let title_app = app.clone();
    // True from the service's sign-in page until sign-in returns to the service:
    // meanwhile an organization's own login page (any HTTPS site) may load.
    let signing_in = Arc::new(AtomicBool::new(false));

    let builder = WebviewBuilder::new(webview_label(&account.id), WebviewUrl::External(url))
        .data_directory(accounts::profile_dir(profiles, &account.id))
        .initialization_script(page_script(service, account))
        .focused(false)
        .on_navigation(move |url| {
            if service.allows(url) {
                signing_in.store(service.is_sign_in(url), Ordering::Relaxed);
                true
            } else if url.scheme() == "https" && signing_in.load(Ordering::Relaxed) {
                true
            } else {
                open_external(url);
                false
            }
        })
        .on_new_window(move |url, _features| {
            if service.allows_popup(&url) {
                // Sign-in and call popups of the service itself, in the same profile.
                NewWindowResponse::Allow
            } else {
                open_external(&url);
                NewWindowResponse::Deny
            }
        })
        .on_document_title_changed(move |_webview, title| {
            set_unread(&title_app, &id, services::unread_from_title(&title));
        });

    let webview = window
        .add_child(builder, LogicalPosition::new(-20000.0, 0.0), LogicalSize::new(800.0, 600.0))
        .map_err(|e| e.to_string())?;
    let _ = webview.set_zoom(account.zoom);
    // The next layout sets this webview's memory level, whatever it recorded before it existed.
    app.state::<AppState>().inner.lock().unwrap().low_memory.remove(&account.id);
    Ok(())
}

#[tauri::command]
fn list_services(webview: Webview) -> Result<&'static [services::Service], String> {
    guard_shell(&webview)?;
    Ok(services::SERVICES)
}

#[tauri::command]
fn list_accounts(app: AppHandle, webview: Webview) -> Result<Snapshot, String> {
    guard_shell(&webview)?;
    Ok(snapshot(&app))
}

// Async: creating a webview from a synchronous command can deadlock on Windows.
#[tauri::command]
async fn add_account(
    app: AppHandle,
    webview: Webview,
    state: State<'_, AppState>,
    service: String,
    label: String,
) -> Result<Snapshot, String> {
    guard(&webview, &state)?;
    let found = services::find(&service).ok_or("unknown service")?;
    let label = accounts::clean_label(&label);
    let account = Account::new(
        accounts::new_id(),
        found.id.to_string(),
        if label.is_empty() { found.name.to_string() } else { label },
    );
    {
        let mut inner = state.inner.lock().unwrap();
        inner.accounts.push(account.clone());
        accounts::save(&state.accounts_file, &inner.accounts)?;
        inner.active = Some(account.id.clone());
        inner.overlay = false;
    }
    let window = app.get_window("main").ok_or("no window")?;
    create_account_webview(&app, &window, &account, &state.profiles)?;
    select(&app, &account.id);
    Ok(snapshot(&app))
}

#[tauri::command]
async fn remove_account(app: AppHandle, webview: Webview, state: State<'_, AppState>, id: String) -> Result<Snapshot, String> {
    guard(&webview, &state)?;
    {
        let mut inner = state.inner.lock().unwrap();
        let before = inner.accounts.len();
        inner.accounts.retain(|a| a.id != id);
        if inner.accounts.len() == before {
            return Err("unknown account".into());
        }
        accounts::save(&state.accounts_file, &inner.accounts)?;
        if inner.active.as_deref() == Some(&id) {
            inner.active = inner.accounts.first().map(|a| a.id.clone());
        }
    }
    if let Some(w) = app.get_webview(&webview_label(&id)) {
        let _ = w.close();
    }
    // Sign the account out for good by deleting its profile. The browser process
    // lets go of the files a moment after the webview closes, so retry for a while.
    let dir = accounts::profile_dir(&state.profiles, &id);
    std::thread::spawn(move || {
        for _ in 0..20 {
            if !dir.exists() || std::fs::remove_dir_all(&dir).is_ok() {
                break;
            }
            std::thread::sleep(Duration::from_millis(500));
        }
    });
    set_unread(&app, &id, 0);
    {
        let mut inner = state.inner.lock().unwrap();
        inner.unread.remove(&id);
        inner.low_memory.remove(&id);
    }
    layout(&app);
    Ok(snapshot(&app))
}

#[tauri::command]
fn select_account(app: AppHandle, webview: Webview, state: State<'_, AppState>, id: String) -> Result<(), String> {
    guard(&webview, &state)?;
    select(&app, &id);
    Ok(())
}

#[tauri::command]
fn reload_account(app: AppHandle, webview: Webview, state: State<'_, AppState>, id: String) -> Result<(), String> {
    guard(&webview, &state)?;
    if !state.inner.lock().unwrap().accounts.iter().any(|a| a.id == id) {
        return Err("unknown account".into());
    }
    if let Some(w) = app.get_webview(&webview_label(&id)) {
        w.reload().map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// Name and icon color; an empty color goes back to the brand color.
#[tauri::command]
fn update_account(
    app: AppHandle,
    webview: Webview,
    state: State<'_, AppState>,
    id: String,
    label: String,
    color: String,
) -> Result<Snapshot, String> {
    guard(&webview, &state)?;
    let label = accounts::clean_label(&label);
    if label.is_empty() {
        return Err("name is empty".into());
    }
    let color = (!color.is_empty()).then_some(color);
    if color.as_deref().is_some_and(|c| !accounts::valid_color(c)) {
        return Err("bad color".into());
    }
    change_account(&app, &id, |accounts, i| {
        accounts[i].label = label;
        accounts[i].color = color;
    })?;
    Ok(snapshot(&app))
}

/// Puts the accounts in the given order, which must name each account once.
#[tauri::command]
fn reorder_accounts(app: AppHandle, webview: Webview, state: State<'_, AppState>, ids: Vec<String>) -> Result<Snapshot, String> {
    guard(&webview, &state)?;
    {
        let mut inner = state.inner.lock().unwrap();
        let mut reordered = Vec::with_capacity(ids.len());
        for id in &ids {
            let account = inner.accounts.iter().find(|a| &a.id == id).ok_or("unknown account")?;
            if reordered.iter().any(|a: &Account| &a.id == id) {
                return Err("duplicate account".into());
            }
            reordered.push(account.clone());
        }
        if reordered.len() != inner.accounts.len() {
            return Err("missing accounts".into());
        }
        inner.accounts = reordered;
        accounts::save(&state.accounts_file, &inner.accounts)?;
    }
    Ok(snapshot(&app))
}

/// Native menus can overlap the service webviews; choices come back through
/// `menus::on_menu_event`.
#[tauri::command]
async fn show_account_menu(app: AppHandle, webview: Webview, state: State<'_, AppState>, id: String) -> Result<(), String> {
    guard(&webview, &state)?;
    let menu = menus::account_menu(&app, &id)?;
    let window = app.get_window("main").ok_or("no window")?;
    window.popup_menu(&menu).map_err(|e| e.to_string())
}

#[tauri::command]
async fn show_dnd_menu(app: AppHandle, webview: Webview, state: State<'_, AppState>) -> Result<(), String> {
    guard(&webview, &state)?;
    let menu = menus::dnd_menu(&app)?;
    let window = app.get_window("main").ok_or("no window")?;
    window.popup_menu(&menu).map_err(|e| e.to_string())
}

#[tauri::command]
fn set_overlay(app: AppHandle, webview: Webview, state: State<'_, AppState>, open: bool) -> Result<(), String> {
    guard_shell(&webview)?;
    state.inner.lock().unwrap().overlay = open;
    layout(&app);
    Ok(())
}

#[tauri::command]
fn update_settings(app: AppHandle, webview: Webview, state: State<'_, AppState>, update: SettingsUpdate) -> Result<Snapshot, String> {
    guard(&webview, &state)?;
    if let Some(on) = update.start_with_windows {
        system::set_autostart(on)?;
    }
    let (theme, panic) = {
        let mut s = state.settings.lock().unwrap();
        let before = (s.theme.clone(), s.panic_shortcut);
        if let Some(theme) = update.theme.filter(|t| matches!(t.as_str(), "system" | "light" | "dark")) {
            s.theme = theme;
        }
        macro_rules! set {
            ($($field:ident),*) => { $( if let Some(v) = update.$field { s.$field = v; } )* };
        }
        set!(show_labels, close_to_tray, start_with_windows, start_minimized, notifications, notification_preview, panic_shortcut);
        if let Some(minutes) = update.lock_idle_minutes {
            s.lock.idle_minutes = minutes.min(24 * 60);
        }
        settings::save(&state.settings_file, &s)?;
        ((s.theme != before.0).then(|| s.theme.clone()), (s.panic_shortcut != before.1).then_some(s.panic_shortcut))
    };
    if let Some(theme) = theme {
        apply_theme(&app, &theme);
    }
    if let Some(on) = panic {
        let shortcuts = app.global_shortcut();
        let _ = if on { shortcuts.register(PANIC_SHORTCUT) } else { shortcuts.unregister(PANIC_SHORTCUT) };
    }
    Ok(snapshot(&app))
}

/// Do Not Disturb for `minutes`; 0 = until turned off; none = off.
#[tauri::command]
fn set_dnd(app: AppHandle, webview: Webview, state: State<'_, AppState>, minutes: Option<u64>) -> Result<Snapshot, String> {
    guard(&webview, &state)?;
    set_dnd_until(&app, minutes.map(|m| if m == 0 { u64::MAX } else { now_secs() + m * 60 }));
    Ok(snapshot(&app))
}

/// Turns the app lock on, or changes its PIN (the current one is needed then).
#[tauri::command]
async fn set_pin(
    app: AppHandle,
    webview: Webview,
    state: State<'_, AppState>,
    current: Option<String>,
    pin: String,
) -> Result<Snapshot, String> {
    guard(&webview, &state)?;
    if !lock::valid_pin(&pin) {
        return Err("The PIN must be 4 to 32 characters.".into());
    }
    let existing = state.settings.lock().unwrap().lock.clone();
    if existing.enabled && !lock::verify(current.as_deref().unwrap_or(""), &existing.salt, &existing.hash) {
        return Err("The current PIN is wrong.".into());
    }
    let (salt, hash) = lock::hash_new(&pin)?;
    {
        let mut s = state.settings.lock().unwrap();
        s.lock.enabled = true;
        s.lock.salt = salt;
        s.lock.hash = hash;
        settings::save(&state.settings_file, &s)?;
    }
    menus::refresh_tray(&app);
    Ok(snapshot(&app))
}

#[tauri::command]
async fn disable_lock(app: AppHandle, webview: Webview, state: State<'_, AppState>, pin: String) -> Result<Snapshot, String> {
    guard(&webview, &state)?;
    {
        let mut s = state.settings.lock().unwrap();
        if !lock::verify(&pin, &s.lock.salt, &s.lock.hash) {
            return Err("Wrong PIN.".into());
        }
        s.lock = settings::LockSettings { idle_minutes: s.lock.idle_minutes, ..Default::default() };
        settings::save(&state.settings_file, &s)?;
    }
    menus::refresh_tray(&app);
    Ok(snapshot(&app))
}

/// Wrong PINs make the next try wait longer: 5 free tries, then 30 s, 60 s, …
#[tauri::command]
async fn unlock(app: AppHandle, webview: Webview, state: State<'_, AppState>, pin: String) -> Result<Snapshot, String> {
    guard_shell(&webview)?;
    let now = now_secs();
    {
        let inner = state.inner.lock().unwrap();
        if !inner.locked {
            return Ok(snapshot(&app));
        }
        if now < inner.unlock_retry_at {
            return Err(format!("Too many tries. Wait {} s.", inner.unlock_retry_at - now));
        }
    }
    let (salt, hash) = {
        let s = state.settings.lock().unwrap();
        (s.lock.salt.clone(), s.lock.hash.clone())
    };
    let ok = lock::verify(&pin, &salt, &hash);
    {
        let mut inner = state.inner.lock().unwrap();
        if ok {
            inner.locked = false;
            inner.failed_unlocks = 0;
        } else {
            inner.failed_unlocks += 1;
            if inner.failed_unlocks >= 5 {
                inner.unlock_retry_at = now + 30 * u64::from(inner.failed_unlocks - 4);
            }
        }
    }
    if !ok {
        return Err("Wrong PIN.".into());
    }
    layout(&app);
    Ok(snapshot(&app))
}

#[tauri::command]
fn lock_now(app: AppHandle, webview: Webview) -> Result<Snapshot, String> {
    guard_shell(&webview)?;
    lock_app(&app);
    Ok(snapshot(&app))
}

#[derive(Serialize)]
struct UpdateInfo {
    version: String,
    notes: Option<String>,
}

#[tauri::command]
async fn check_update(app: AppHandle, webview: Webview, state: State<'_, AppState>) -> Result<Option<UpdateInfo>, String> {
    guard(&webview, &state)?;
    let update = app.updater().map_err(|e| e.to_string())?.check().await.map_err(|e| e.to_string())?;
    Ok(update.map(|u| UpdateInfo { version: u.version, notes: u.body }))
}

/// Downloads the update, checks its signature against the app's public key,
/// and installs it; on Windows the installer closes and restarts the app.
#[tauri::command]
async fn install_update(app: AppHandle, webview: Webview, state: State<'_, AppState>) -> Result<(), String> {
    guard(&webview, &state)?;
    let update = app.updater().map_err(|e| e.to_string())?.check().await.map_err(|e| e.to_string())?;
    let Some(update) = update else { return Ok(()) };
    update.download_and_install(|_, _| {}, || {}).await.map_err(|e| e.to_string())?;
    app.restart();
}

/// The one command a service page may call (capabilities/services.json): show
/// a notification for the account whose webview is calling.
#[tauri::command]
fn notify(app: AppHandle, webview: Webview, state: State<'_, AppState>, id: u32, title: String, body: String) -> Result<(), String> {
    let account_id = webview.label().strip_prefix("acc-").ok_or("forbidden")?.to_string();
    let (enabled, preview) = {
        let s = state.settings.lock().unwrap();
        (s.notifications && !s.dnd_active(), s.notification_preview)
    };
    if !enabled {
        return Ok(());
    }
    let (label, locked) = {
        let mut inner = state.inner.lock().unwrap();
        let account = inner.accounts.iter().find(|a| a.id == account_id).ok_or("unknown account")?;
        if !account.notify {
            return Ok(());
        }
        let label = account.label.clone();
        // Not while you're looking at that account.
        if inner.focused && !inner.locked && !inner.overlay && inner.active.as_deref() == Some(&account_id) {
            return Ok(());
        }
        let now = now_millis();
        let log = inner.notify_log.entry(account_id.clone()).or_default();
        log.retain(|t| now - t < 10_000);
        if log.len() >= NOTIFY_BURST {
            return Ok(());
        }
        log.push_back(now);
        (label, inner.locked)
    };
    let short = |s: &str, n: usize| s.chars().filter(|c| !c.is_control()).take(n).collect::<String>();
    // Locked, or with previews off, a notification only says which account has news.
    let (title, body) = if locked || !preview {
        (label, "New message".to_string())
    } else {
        (format!("{} · {}", short(&title, 100), label), short(&body, 300))
    };
    let click_app = app.clone();
    system::toast(&system::app_id(&app.config().identifier), &title, &body, move || {
        show_window(&click_app);
        if click_app.state::<AppState>().inner.lock().unwrap().locked {
            return;
        }
        select(&click_app, &account_id);
        call_shell(&click_app, "window.masterChat && window.masterChat.refresh()".into());
        if let Some(w) = click_app.get_webview(&webview_label(&account_id)) {
            let _ = w.eval(format!("window.__mcNotificationClicked && window.__mcNotificationClicked({id})"));
        }
    })
}

fn on_panic_shortcut(app: &AppHandle) {
    let Some(window) = app.get_window("main") else { return };
    let showing = window.is_visible().unwrap_or(false) && !window.is_minimized().unwrap_or(false);
    if showing {
        lock_app(app);
        let _ = window.hide();
    } else {
        show_window(app);
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(
            tauri_plugin_global_shortcut::Builder::new()
                .with_handler(|app, _shortcut, event| {
                    if event.state() == ShortcutState::Pressed {
                        on_panic_shortcut(app);
                    }
                })
                .build(),
        )
        .plugin(tauri_plugin_updater::Builder::new().build())
        .invoke_handler(tauri::generate_handler![
            list_services,
            list_accounts,
            add_account,
            remove_account,
            select_account,
            reload_account,
            update_account,
            reorder_accounts,
            show_account_menu,
            show_dnd_menu,
            set_overlay,
            update_settings,
            set_dnd,
            set_pin,
            disable_lock,
            unlock,
            lock_now,
            check_update,
            install_update,
            notify
        ])
        .on_menu_event(|app, event| menus::on_menu_event(app, event.id().as_ref()))
        .setup(|app| {
            let config_dir = app.path().app_config_dir()?;
            let accounts_file = config_dir.join("accounts.json");
            let settings_file = config_dir.join("settings.json");
            let profiles = app.path().app_local_data_dir()?.join("profiles");
            let accounts = accounts::load(&accounts_file);
            let settings = settings::load(&settings_file);
            let active = accounts.first().map(|a| a.id.clone());
            let autostarted = std::env::args().any(|a| a == "--autostart");
            let start_hidden = autostarted && settings.start_minimized;
            let theme = settings.theme.clone();
            let panic = settings.panic_shortcut;
            app.manage(AppState {
                inner: Mutex::new(Inner {
                    accounts: accounts.clone(),
                    active: active.clone(),
                    overlay: false,
                    unread: HashMap::new(),
                    locked: settings.lock.enabled,
                    failed_unlocks: 0,
                    unlock_retry_at: 0,
                    focused: true,
                    notify_log: HashMap::new(),
                    low_memory: HashMap::new(),
                }),
                settings: Mutex::new(settings),
                accounts_file,
                settings_file,
                profiles: profiles.clone(),
            });

            let window = WindowBuilder::new(app, "main")
                .title("Master Chat")
                .inner_size(1280.0, 820.0)
                .min_inner_size(720.0, 480.0)
                .visible(!start_hidden)
                .build()?;
            let handle = app.handle().clone();
            apply_theme(&handle, &theme);
            let size = window.inner_size()?.to_logical::<f64>(window.scale_factor()?);

            // The shell may only ever show the app's own pages: the bundled ones, or in
            // `tauri dev` the dev server the CLI serves them from.
            let dev_origin = app.config().build.dev_url.as_ref().filter(|_| cfg!(debug_assertions)).map(|u| u.origin());
            let shell = WebviewBuilder::new(SHELL, WebviewUrl::App("index.html".into())).on_navigation(move |url| {
                let ok = url.scheme() == "tauri"
                    || url.host_str() == Some("tauri.localhost")
                    || dev_origin.as_ref() == Some(&url.origin());
                if !ok {
                    eprintln!("blocked shell navigation to {url}");
                }
                ok
            });
            window.add_child(shell, LogicalPosition::new(0.0, 0.0), size)?;

            // The account on screen loads first; the rest follow a few seconds apart,
            // since each is a whole browser starting up, so the window stays usable.
            let (first, rest): (Vec<_>, Vec<_>) = accounts.into_iter().partition(|a| Some(&a.id) == active.as_ref());
            for account in &first {
                if let Err(e) = create_account_webview(&handle, &window, account, &profiles) {
                    eprintln!("could not open {}: {e}", account.label);
                }
            }
            layout(&handle);
            let (later_app, later_window, later_profiles) = (handle.clone(), window.clone(), profiles.clone());
            std::thread::spawn(move || {
                std::thread::sleep(Duration::from_secs(3));
                for account in rest {
                    if let Err(e) = create_account_webview(&later_app, &later_window, &account, &later_profiles) {
                        eprintln!("could not open {}: {e}", account.label);
                    }
                    layout(&later_app);
                    std::thread::sleep(Duration::from_secs(4));
                }
            });

            let tray_app = handle.clone();
            TrayIconBuilder::with_id(menus::TRAY_ID)
                .icon(app.default_window_icon().cloned().ok_or("no icon")?)
                .tooltip("Master Chat")
                .menu(&menus::tray_menu(&handle)?)
                .show_menu_on_left_click(false)
                .on_tray_icon_event(move |_tray, event| {
                    if let TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, .. } = event {
                        show_window(&tray_app);
                    }
                })
                .build(app)?;

            if panic {
                if let Err(e) = app.global_shortcut().register(PANIC_SHORTCUT) {
                    eprintln!("could not register {PANIC_SHORTCUT}: {e}");
                }
            }

            // Locks the app after the set time without keyboard or mouse input.
            let idle_app = handle.clone();
            std::thread::spawn(move || loop {
                std::thread::sleep(Duration::from_secs(15));
                let state = idle_app.state::<AppState>();
                let minutes = {
                    let s = state.settings.lock().unwrap();
                    if s.lock.enabled { s.lock.idle_minutes } else { 0 }
                };
                if minutes > 0 && !state.inner.lock().unwrap().locked && system::idle_seconds() >= u64::from(minutes) * 60 {
                    lock_app(&idle_app);
                }
            });

            let events_app = handle.clone();
            window.on_window_event(move |event| match event {
                WindowEvent::Resized(_) | WindowEvent::ScaleFactorChanged { .. } => layout(&events_app),
                WindowEvent::Focused(focused) => {
                    events_app.state::<AppState>().inner.lock().unwrap().focused = *focused;
                }
                WindowEvent::CloseRequested { api, .. } if events_app.state::<AppState>().settings.lock().unwrap().close_to_tray => {
                    api.prevent_close();
                    if let Some(w) = events_app.get_window("main") {
                        let _ = w.hide();
                    }
                }
                _ => {}
            });
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running Master Chat");
}
