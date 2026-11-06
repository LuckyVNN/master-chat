mod accounts;
mod badge;
mod google_domains;
mod horoscope;
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
    collections::{HashMap, HashSet, VecDeque},
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, AtomicU32, Ordering},
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
/// Height of the bar along the bottom that lists popped-out chats, when there are any.
const DOCK: f64 = 44.0;
/// A popped-out chat's page is kept at least this wide (zoomed out if need be):
/// narrower, Facebook shows only its chat list.
const PAGE_MIN: f64 = 600.0;
/// Label of the local UI webview, the only one allowed to manage the app.
const SHELL: &str = "shell";
/// Hides the window (and locks the app, if the lock is on) from anywhere.
const PANIC_SHORTCUT: &str = "ctrl+alt+h";
/// A service may show at most this many notifications per 10 seconds.
const NOTIFY_BURST: usize = 5;
/// The dashboard lists this many of the latest messages.
const RECENT_MAX: usize = 30;
/// Browser options for a real-time account: Chromium slows a hidden page's
/// timers, down to once a minute after five minutes, which delays new messages.
/// wry's own defaults (the first three features) are kept.
/// One sample of each account's unread count this often, for a day.
const HISTORY_STEP: Duration = Duration::from_secs(15 * 60);
const HISTORY_LEN: usize = 96;
/// Away this long (seconds) or more gets a summary of what came in meanwhile.
const AWAY_MIN: u64 = 10 * 60;
const REALTIME_ARGS: &str = "--disable-features=msWebOOUI,msPdfOOUI,msSmartScreenProtection,IntensiveWakeUpThrottling \
    --disable-background-timer-throttling --disable-renderer-backgrounding --disable-backgrounding-occluded-windows";

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
    /// The latest messages, newest first, for the dashboard. Only ever in memory.
    recent: VecDeque<Recent>,
    /// When each account's page last started loading (ms): its unread count
    /// settles from 0 to the real number then, which isn't news.
    loaded_at: HashMap<String, u128>,
    /// Chats popped out into windows of their own, in the order opened.
    pops: Vec<Pop>,
    pop_seq: u32,
    /// Accounts whose page shows a sign-in screen.
    signed_out: HashSet<String>,
    /// Each account's unread count every HISTORY_STEP, oldest first, for a day.
    history: HashMap<String, VecDeque<u32>>,
    /// When you stepped away (the window lost focus, or no input), if you have.
    left_at: Option<u64>,
    /// The start of your last absence of AWAY_MIN or more, for the overview's summary.
    away_since: Option<u64>,
    /// An update the app found on its own, waiting for your answer.
    update: Option<UpdateInfo>,
    /// Versions already offered this run, so each is asked about once.
    offered: HashSet<String>,
}

/// A popped-out chat: a small window on one conversation, signed in through
/// its account's browser profile.
#[derive(Serialize, Clone)]
struct Pop {
    label: String,
    account: String,
    title: String,
    #[serde(skip)]
    url: String,
}

impl Inner {
    /// The count the badges and totals show: none for an account set to hide it.
    fn shown_unread(&self, account: &Account) -> u32 {
        if account.hide_unread {
            0
        } else {
            self.unread.get(&account.id).copied().unwrap_or(0)
        }
    }
}

#[derive(Serialize, Clone)]
struct Recent {
    account: String,
    /// The page's notification id, to open that chat when clicked.
    id: u32,
    /// Empty when message previews are off.
    title: String,
    body: String,
    at: u64,
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
    realtime: bool,
    hide_unread: bool,
    signed_out: bool,
    /// Its page is on a single chat, which can be popped out.
    on_chat: bool,
    history: Vec<u32>,
}

#[derive(Serialize)]
struct Snapshot {
    accounts: Vec<AccountView>,
    active: Option<String>,
    locked: bool,
    settings: SettingsView,
    version: String,
    recent: Vec<Recent>,
    pops: Vec<Pop>,
    away_since: Option<u64>,
    update: Option<UpdateInfo>,
}

pub(crate) fn now_secs() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

pub(crate) fn now_millis() -> u128 {
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
/// Sign-in and call popups of the service itself open as app windows on the
/// same profile, held to the service's own sites; anything else goes to the
/// default browser.
fn service_popup(
    app: &AppHandle,
    service: &'static services::Service,
    url: &Url,
    features: tauri::webview::NewWindowFeatures,
) -> NewWindowResponse<tauri::Wry> {
    static SEQ: AtomicU32 = AtomicU32::new(0);
    if !service.allows_popup(url) {
        open_external(url);
        return NewWindowResponse::Deny;
    }
    let label = format!("popup-{}", SEQ.fetch_add(1, Ordering::Relaxed));
    let blank = Url::parse("about:blank").expect("valid url");
    let nested = app.clone();
    let built = tauri::WebviewWindowBuilder::new(app, label, WebviewUrl::External(blank))
        .title(service.name)
        .inner_size(960.0, 640.0)
        .window_features(features)
        .on_navigation(move |url| {
            if service.allows(url) {
                true
            } else {
                open_external(url);
                false
            }
        })
        .on_new_window(move |url, features| service_popup(&nested, service, &url, features))
        .on_document_title_changed(|window, title| {
            let _ = window.set_title(&title);
        })
        .build();
    match built {
        Ok(window) => NewWindowResponse::Create { window },
        Err(_) => NewWindowResponse::Deny,
    }
}

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
                unread: inner.shown_unread(a),
                notify: a.notify,
                realtime: a.realtime,
                hide_unread: a.hide_unread,
                signed_out: inner.signed_out.contains(&a.id),
                on_chat: services::find(&a.service).is_some_and(|s| {
                    app.get_webview(&webview_label(&a.id)).and_then(|w| w.url().ok()).is_some_and(|u| s.is_chat(&u))
                }),
                history: inner.history.get(&a.id).map(|h| h.iter().copied().collect()).unwrap_or_default(),
            })
            .collect()
    };
    Snapshot {
        accounts,
        active: if inner.locked { None } else { inner.active.clone() },
        locked: inner.locked,
        settings,
        version: app.package_info().version.to_string(),
        recent: if inner.locked { Vec::new() } else { inner.recent.iter().cloned().collect() },
        pops: if inner.locked { Vec::new() } else { inner.pops.clone() },
        away_since: if inner.locked { None } else { inner.away_since },
        update: if inner.locked { None } else { inner.update.clone() },
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
    // Minimized or in the tray, no page is on screen, so each may notify.
    let window_shown = window.is_visible().unwrap_or(true) && !window.is_minimized().unwrap_or(false);

    if let Some(shell) = app.get_webview(SHELL) {
        let _ = shell.set_position(LogicalPosition::new(0.0, 0.0));
        let _ = shell.set_size(size);
    }

    // Collect first so no webview call runs while the lock is held. The memory
    // level is only sent when it changes, not on every resize.
    let (dock, visible): (f64, Vec<(String, bool, Option<bool>)>) = {
        let state = app.state::<AppState>();
        let mut inner = state.inner.lock().unwrap();
        let dock = if inner.pops.is_empty() || inner.locked { 0.0 } else { DOCK };
        let hidden = inner.overlay || inner.locked || !window_shown;
        let shown: Vec<(String, bool, bool)> = inner
            .accounts
            .iter()
            .map(|a| (a.id.clone(), !hidden && inner.active.as_deref() == Some(&a.id), a.realtime))
            .collect();
        let visible = shown
            .into_iter()
            .map(|(id, show, realtime)| {
                // Real-time accounts keep their memory, so they answer at once.
                let low = !show && !realtime;
                let changed = inner.low_memory.insert(id.clone(), low) != Some(low);
                (webview_label(&id), show, changed.then_some(low))
            })
            .collect();
        (dock, visible)
    };
    for (label, show, memory) in visible {
        let Some(webview) = app.get_webview(&label) else { continue };
        if let Some(low) = memory {
            system::set_low_memory(&webview, low);
        }
        if show {
            let _ = webview.set_position(LogicalPosition::new(SIDEBAR, 0.0));
            let _ = webview.set_size(LogicalSize::new((size.width - SIDEBAR).max(1.0), (size.height - dock).max(1.0)));
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
        inner.accounts.iter().map(|a| inner.shown_unread(a)).sum()
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
    let shown = {
        let state = app.state::<AppState>();
        let mut inner = state.inner.lock().unwrap();
        if inner.unread.get(id) == Some(&count) {
            return;
        }
        let rose = inner.unread.insert(id.to_string(), count).is_some_and(|before| count > before);
        let now = now_millis();
        let settled = inner.loaded_at.get(id).is_none_or(|&t| now - t > 30_000);
        let describes = inner
            .accounts
            .iter()
            .find(|a| a.id == id)
            .and_then(|a| services::find(&a.service))
            .is_some_and(|s| s.describes_messages);
        if rose && settled && !describes {
            notice_if_silent(app, id.to_string(), now);
        }
        inner.accounts.iter().find(|a| a.id == id).map_or(0, |a| inner.shown_unread(a))
    };
    show_total_unread(app);
    let id = serde_json::to_string(id).unwrap_or_default();
    call_shell(app, format!("window.masterChat && window.masterChat.unread({id}, {shown})"));
}

pub(crate) fn show_window(app: &AppHandle) {
    if let Some(window) = app.get_window("main") {
        let _ = window.show();
        let _ = window.unminimize();
        let _ = window.set_focus();
    }
    layout(app);
}

fn hide_window(app: &AppHandle) {
    if let Some(window) = app.get_window("main") {
        let _ = window.hide();
    }
    layout(app);
}

fn apply_theme(app: &AppHandle, theme: &str) {
    let theme = match theme {
        // Sakura is the sidebar's own palette; the services show their light look.
        "light" | "sakura" => Some(Theme::Light),
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
    let id = serde_json::to_string(id).unwrap_or_default();
    call_shell(app, format!("window.masterChat && window.masterChat.selected({id})"));
}

pub(crate) fn lock_app(app: &AppHandle) {
    let state = app.state::<AppState>();
    if !state.settings.lock().unwrap().lock.enabled {
        return;
    }
    state.inner.lock().unwrap().locked = true;
    hide_pops(app);
    layout(app);
    refresh_shell(app);
}

fn pop_window(app: &AppHandle, label: &str) -> Option<Window> {
    label.starts_with("pop-").then(|| app.get_window(label)).flatten()
}

/// Popped-out chats hide with the app (lock, panic shortcut).
fn hide_pops(app: &AppHandle) {
    let labels: Vec<String> = app.state::<AppState>().inner.lock().unwrap().pops.iter().map(|p| p.label.clone()).collect();
    for label in labels {
        if let Some(w) = pop_window(app, &label) {
            let _ = w.hide();
        }
    }
}

/// Opens the conversation an account is showing in a small window of its own,
/// on top of other windows. It uses the account's browser profile (so it's
/// signed in) and the same rules for where it may go.
pub(crate) fn pop_out(app: &AppHandle, account_id: &str) -> Result<(), String> {
    let url = app.get_webview(&webview_label(account_id)).ok_or("not open")?.url().map_err(|e| e.to_string())?;
    pop_out_url(app, account_id, url)
}

fn pop_out_url(app: &AppHandle, account_id: &str, url: Url) -> Result<(), String> {
    let state = app.state::<AppState>();
    let (account, n) = {
        let mut inner = state.inner.lock().unwrap();
        if inner.locked {
            return Err("locked".into());
        }
        let account = inner.accounts.iter().find(|a| a.id == account_id).cloned().ok_or("unknown account")?;
        inner.pop_seq += 1;
        (account, inner.pop_seq)
    };
    let service = services::find(&account.service).ok_or("unknown service")?;
    if !service.is_chat(&url) {
        return Err("open a conversation first".into());
    }
    if let Some(open) = state.inner.lock().unwrap().pops.iter().find(|p| p.account == account.id && p.url == url.as_str()) {
        let _ = app.get_window(&open.label).map(|w| (w.show(), w.unminimize(), w.set_focus()));
        return Ok(());
    }
    let label = format!("pop-{n}");
    // Stacked up from the bottom right of the screen the app is on.
    let (mut x, mut y) = (60.0, 60.0);
    if let Some(m) = app.get_window("main").and_then(|w| w.current_monitor().ok().flatten()) {
        let scale = m.scale_factor();
        let (pos, size) = (m.position().to_logical::<f64>(scale), m.size().to_logical::<f64>(scale));
        let shift = f64::from((n - 1) % 6) * 28.0;
        x = pos.x + size.width - 480.0 - 24.0 - shift;
        y = pos.y + size.height - 660.0 - 64.0 - shift;
    }
    let title_app = app.clone();
    let title_label = label.clone();
    let mut builder = tauri::WebviewWindowBuilder::new(app, &label, WebviewUrl::External(url.clone()))
        .title(service.name)
        .inner_size(480.0, 640.0)
        .min_inner_size(320.0, 360.0)
        .position(x, y)
        .always_on_top(true)
        // Its button is in the app's chat bar, not on the Windows taskbar.
        .skip_taskbar(true)
        .data_directory(accounts::profile_dir(&state.profiles, &account.id))
        .initialization_script(page_script(service, &account, true))
        .on_navigation(move |url| {
            if service.allows(url) {
                true
            } else {
                open_external(url);
                false
            }
        })
        .on_new_window({
            let app = app.clone();
            move |url, features| service_popup(&app, service, &url, features)
        })
        .on_document_title_changed(move |window, title| {
            {
                let state = title_app.state::<AppState>();
                let mut inner = state.inner.lock().unwrap();
                let Some(pop) = inner.pops.iter_mut().find(|p| p.label == title_label) else { return };
                pop.title = title.clone();
            }
            // The page's title is the person's name (or the channel's).
            let _ = window.set_title(&title);
            refresh_shell(&title_app);
        });
    // The same browser options as the account's page, as they share its profile.
    if account.realtime {
        builder = builder.additional_browser_args(REALTIME_ARGS);
    }
    let window = builder.build().map_err(|e| e.to_string())?;
    state.inner.lock().unwrap().pops.push(Pop { label: label.clone(), account: account.id.clone(), title: String::new(), url: url.to_string() });
    let closed_app = app.clone();
    // Facebook renders no conversation below 600px of page width, so a narrower
    // window is zoomed out to keep the page that wide.
    let fit = move |w: &tauri::WebviewWindow| {
        let width = w.inner_size().ok().zip(w.scale_factor().ok()).map_or(PAGE_MIN, |(s, f)| s.to_logical::<f64>(f).width);
        let _ = w.set_zoom(account.zoom * (width / PAGE_MIN).min(1.0));
    };
    fit(&window);
    let tuck = window.clone();
    window.on_window_event(move |event| match event {
        WindowEvent::Destroyed => {
            closed_app.state::<AppState>().inner.lock().unwrap().pops.retain(|p| p.label != label);
            layout(&closed_app);
            refresh_shell(&closed_app);
        }
        // Minimizing tucks the chat into the chat bar; its chip brings it back.
        WindowEvent::Resized(_) if tuck.is_minimized().unwrap_or(false) => {
            let _ = tuck.hide();
            let _ = tuck.unminimize();
        }
        WindowEvent::Resized(_) | WindowEvent::ScaleFactorChanged { .. } => fit(&tuck),
        _ => {}
    });
    layout(app);
    refresh_shell(app);
    Ok(())
}

/// A chip in the chat bar: brings that chat's window up, or tucks it away if
/// it's already in front.
#[tauri::command]
fn toggle_pop(webview: Webview, state: State<'_, AppState>, app: AppHandle, label: String) -> Result<(), String> {
    guard(&webview, &state)?;
    let window = pop_window(&app, &label).ok_or("no such chat")?;
    let in_front = window.is_visible().unwrap_or(false) && window.is_focused().unwrap_or(false);
    if in_front {
        window.hide().map_err(|e| e.to_string())
    } else {
        let _ = window.show();
        let _ = window.unminimize();
        window.set_focus().map_err(|e| e.to_string())
    }
}

#[tauri::command]
fn close_pop(webview: Webview, state: State<'_, AppState>, app: AppHandle, label: String) -> Result<(), String> {
    guard(&webview, &state)?;
    pop_window(&app, &label).ok_or("no such chat")?.close().map_err(|e| e.to_string())
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
    show_total_unread(app);
    refresh_shell(app);
    Ok(())
}

/// The scripts a service's page gets at document start: the notifications
/// shim everywhere, the account's options, and the service's own restyling.
fn page_script(service: &services::Service, account: &Account, popout: bool) -> String {
    let options = serde_json::json!({ "blockTyping": account.block_typing, "popout": popout });
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

    let mut builder = WebviewBuilder::new(webview_label(&account.id), WebviewUrl::External(url))
        .data_directory(accounts::profile_dir(profiles, &account.id))
        .initialization_script(page_script(service, account, false))
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
        .on_new_window({
            let app = app.clone();
            move |url, features| service_popup(&app, service, &url, features)
        })
        .on_document_title_changed(move |_webview, title| {
            set_unread(&title_app, &id, services::unread_from_title(&title));
        });
    if account.realtime {
        builder = builder.additional_browser_args(REALTIME_ARGS);
    }

    let webview = window
        .add_child(builder, LogicalPosition::new(-20000.0, 0.0), LogicalSize::new(800.0, 600.0))
        .map_err(|e| e.to_string())?;
    let _ = webview.set_zoom(account.zoom);
    {
        let state = app.state::<AppState>();
        let mut inner = state.inner.lock().unwrap();
        // The next layout sets this webview's memory level, whatever it recorded before it existed.
        inner.low_memory.remove(&account.id);
        inner.loaded_at.insert(account.id.clone(), now_millis());
    }
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
    let pops: Vec<String> = state.inner.lock().unwrap().pops.iter().filter(|p| p.account == id).map(|p| p.label.clone()).collect();
    for label in pops {
        if let Some(w) = pop_window(&app, &label) {
            let _ = w.close();
        }
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
        inner.recent.retain(|r| r.account != id);
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
        state.inner.lock().unwrap().loaded_at.insert(id.clone(), now_millis());
        w.reload().map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// Notifications on or off for one account (the overview's tick; also in its menu).
#[tauri::command]
fn set_notify(app: AppHandle, webview: Webview, state: State<'_, AppState>, id: String, on: bool) -> Result<Snapshot, String> {
    guard(&webview, &state)?;
    change_account(&app, &id, |accounts, i| accounts[i].notify = on)?;
    Ok(snapshot(&app))
}

/// Real-time on or off. Browser options are fixed when a page's browser starts,
/// so the account's page is closed and opened again.
#[tauri::command]
async fn set_realtime(app: AppHandle, webview: Webview, state: State<'_, AppState>, id: String, on: bool) -> Result<Snapshot, String> {
    guard(&webview, &state)?;
    change_account(&app, &id, |accounts, i| accounts[i].realtime = on)?;
    let account = state.inner.lock().unwrap().accounts.iter().find(|a| a.id == id).cloned().ok_or("unknown account")?;
    if let Some(w) = app.get_webview(&webview_label(&id)) {
        let _ = w.close();
    }
    let window = app.get_window("main").ok_or("no window")?;
    let (profiles, reopen_app) = (state.profiles.clone(), app.clone());
    std::thread::spawn(move || {
        // The old browser lets go of the profile a moment after closing.
        for _ in 0..20 {
            std::thread::sleep(Duration::from_millis(500));
            if reopen_app.get_webview(&webview_label(&account.id)).is_none()
                && create_account_webview(&reopen_app, &window, &account, &profiles).is_ok()
            {
                layout(&reopen_app);
                return;
            }
        }
        eprintln!("could not reopen {}", account.label);
    });
    Ok(snapshot(&app))
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
        if let Some(theme) = update.theme.filter(|t| matches!(t.as_str(), "system" | "light" | "dark" | "sakura")) {
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

#[derive(Serialize, Clone)]
struct UpdateInfo {
    version: String,
    notes: Option<String>,
}

/// Looks for an update on its own, and offers it unless you skipped that
/// version or asked to be reminded later: in the app, and as a notification.
async fn look_for_update(app: &AppHandle) {
    let Ok(updater) = app.updater() else { return };
    let Ok(Some(found)) = updater.check().await else { return };
    let state = app.state::<AppState>();
    {
        let settings = state.settings.lock().unwrap();
        if settings.skip_version.as_deref() == Some(found.version.as_str()) || now_secs() < settings.update_remind_at {
            return;
        }
    }
    {
        let mut inner = state.inner.lock().unwrap();
        if !inner.offered.insert(found.version.clone()) {
            return;
        }
        inner.update = Some(UpdateInfo { version: found.version.clone(), notes: found.body.clone() });
    }
    refresh_shell(app);
    let click_app = app.clone();
    let _ = system::toast(
        &app.config().identifier,
        "Master Chat update available",
        &format!("Version {} is ready to install.", found.version),
        move || show_window(&click_app),
    );
}

/// Oha Asa's horoscope for today, in English (see horoscope.rs).
#[tauri::command]
async fn horoscope(webview: Webview, state: State<'_, AppState>, refresh: bool) -> Result<horoscope::Horoscope, String> {
    guard(&webview, &state)?;
    horoscope::get(refresh).await
}

/// Your answer to an offered update: "later" asks again in a day, "skip"
/// never offers this version again, "close" just puts it away for now.
#[tauri::command]
fn answer_update(app: AppHandle, webview: Webview, state: State<'_, AppState>, version: String, choice: String) -> Result<(), String> {
    guard(&webview, &state)?;
    {
        let mut settings = state.settings.lock().unwrap();
        match choice.as_str() {
            "later" => settings.update_remind_at = now_secs() + 24 * 3600,
            "skip" => settings.skip_version = Some(version),
            "close" => {}
            _ => return Err("unknown choice".into()),
        }
        settings::save(&state.settings_file, &settings)?;
    }
    state.inner.lock().unwrap().update = None;
    refresh_shell(&app);
    Ok(())
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

/// Shows the window and opens the chat a notification was about.
fn open_message(app: &AppHandle, account_id: &str, id: u32) {
    show_window(app);
    if app.state::<AppState>().inner.lock().unwrap().locked {
        return;
    }
    select(app, account_id);
    refresh_shell(app);
    if let Some(w) = app.get_webview(&webview_label(account_id)) {
        let _ = w.eval(format!("window.__mcNotificationClicked && window.__mcNotificationClicked({id})"));
    }
}

/// A service page reports whether it shows a sign-in screen (notify.js).
#[tauri::command]
fn page_state(app: AppHandle, webview: Webview, signed_out: bool) -> Result<(), String> {
    let account_id = webview.label().strip_prefix("acc-").ok_or("forbidden")?.to_string();
    let changed = {
        let state = app.state::<AppState>();
        let mut inner = state.inner.lock().unwrap();
        if !inner.accounts.iter().any(|a| a.id == account_id) {
            return Err("unknown account".into());
        }
        if signed_out { inner.signed_out.insert(account_id) } else { inner.signed_out.remove(&account_id) }
    };
    if changed {
        refresh_shell(&app);
    }
    Ok(())
}

/// The overview card's buttons: the same as the right-click menu's items.
#[tauri::command]
fn account_action(webview: Webview, state: State<'_, AppState>, app: AppHandle, id: String, action: String) -> Result<(), String> {
    guard(&webview, &state)?;
    if !matches!(action.as_str(), "reload" | "pop" | "read_all") {
        return Err("unknown action".into());
    }
    if !state.inner.lock().unwrap().accounts.iter().any(|a| a.id == id) {
        return Err("unknown account".into());
    }
    menus::on_account_action(&app, &action, &id);
    Ok(())
}

/// Reply from the overview: opens the message's chat, then pops it out.
#[tauri::command]
fn reply_recent(app: AppHandle, webview: Webview, state: State<'_, AppState>, account: String, id: u32) -> Result<(), String> {
    guard(&webview, &state)?;
    if !state.inner.lock().unwrap().accounts.iter().any(|a| a.id == account) {
        return Err("unknown account".into());
    }
    open_message(&app, &account, id);
    std::thread::spawn(move || {
        // Give the page a moment to open the chat.
        for _ in 0..10 {
            std::thread::sleep(Duration::from_millis(500));
            if pop_out(&app, &account).is_ok() {
                return;
            }
        }
    });
    Ok(())
}

#[tauri::command]
fn dismiss_away(app: AppHandle, webview: Webview, state: State<'_, AppState>) -> Result<(), String> {
    guard(&webview, &state)?;
    state.inner.lock().unwrap().away_since = None;
    refresh_shell(&app);
    Ok(())
}

/// Notes when you step away from the app (it loses focus, or no input for a
/// while) and, when you're back after AWAY_MIN or more, when you left.
fn check_presence(app: &AppHandle) {
    let idle = system::idle_seconds();
    let back = {
        let state = app.state::<AppState>();
        let mut inner = state.inner.lock().unwrap();
        let present = inner.focused && !inner.locked && idle < 60;
        let now = now_secs();
        match (present, inner.left_at) {
            (false, None) => {
                inner.left_at = Some(now.saturating_sub(if inner.focused { idle } else { 0 }));
                false
            }
            (true, Some(left)) => {
                inner.left_at = None;
                if now.saturating_sub(left) >= AWAY_MIN {
                    inner.away_since = Some(left);
                    true
                } else {
                    false
                }
            }
            _ => false,
        }
    };
    if back {
        refresh_shell(app);
    }
}

/// Opens a message from the dashboard's recent list.
#[tauri::command]
fn open_recent(app: AppHandle, webview: Webview, state: State<'_, AppState>, account: String, id: u32) -> Result<(), String> {
    guard(&webview, &state)?;
    if !state.inner.lock().unwrap().accounts.iter().any(|a| a.id == account) {
        return Err("unknown account".into());
    }
    open_message(&app, &account, id);
    Ok(())
}

#[tauri::command]
fn clear_recent(app: AppHandle, webview: Webview, state: State<'_, AppState>) -> Result<Snapshot, String> {
    guard(&webview, &state)?;
    state.inner.lock().unwrap().recent.clear();
    Ok(snapshot(&app))
}

/// The one command a service page may call (capabilities/services.json): note
/// a message for the dashboard and maybe show a notification, for the account
/// whose webview is calling.
#[tauri::command]
fn notify(app: AppHandle, webview: Webview, id: u32, title: String, body: String) -> Result<(), String> {
    let account_id = webview.label().strip_prefix("acc-").ok_or("forbidden")?.to_string();
    deliver(&app, account_id, id, &title, &body)
}

/// Some pages don't notify (Web Push, which WebView2 lacks, or their own setting
/// is off). If an account's unread count went up and its page sent nothing
/// within a few seconds, a plain notice goes out instead.
fn notice_if_silent(app: &AppHandle, account_id: String, since: u128) {
    let app = app.clone();
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_secs(6));
        let state = app.state::<AppState>();
        let silent = {
            let inner = state.inner.lock().unwrap();
            let notified = inner.notify_log.get(&account_id).and_then(|log| log.back()).is_some_and(|&t| t >= since);
            let looking = inner.focused && !inner.locked && !inner.overlay && inner.active.as_deref() == Some(&account_id);
            !notified && !looking
        };
        if silent {
            let _ = deliver(&app, account_id, 0, "New message", "");
        }
    });
}

/// Records a message for the dashboard and shows a notification unless muted,
/// in Do Not Disturb, or you're looking at that account. `id` is the page's
/// notification id (0 for none), used to open that chat when clicked.
fn deliver(app: &AppHandle, account_id: String, id: u32, title: &str, body: &str) -> Result<(), String> {
    let state = app.state::<AppState>();
    let (enabled, preview) = {
        let s = state.settings.lock().unwrap();
        (s.notifications && !s.dnd_active(), s.notification_preview)
    };
    let short = |s: &str, n: usize| s.chars().filter(|c| !c.is_control()).take(n).collect::<String>();
    let (label, locked, show) = {
        let mut inner = state.inner.lock().unwrap();
        let account = inner.accounts.iter().find(|a| a.id == account_id).ok_or("unknown account")?;
        let (label, muted) = (account.label.clone(), !account.notify);
        let now = now_millis();
        let log = inner.notify_log.entry(account_id.clone()).or_default();
        log.retain(|t| now - t < 10_000);
        if log.len() >= NOTIFY_BURST {
            return Ok(());
        }
        log.push_back(now);
        // The dashboard lists it even when muted or in Do Not Disturb, so you
        // can see what you missed.
        let (recent_title, recent_body) = if preview { (short(title, 100), short(body, 300)) } else { Default::default() };
        inner.recent.push_front(Recent { account: account_id.clone(), id, title: recent_title, body: recent_body, at: now_secs() });
        inner.recent.truncate(RECENT_MAX);
        // No pop-up while you're looking at that account.
        let looking = inner.focused && !inner.locked && !inner.overlay && inner.active.as_deref() == Some(&account_id);
        (label, inner.locked, enabled && !muted && !looking)
    };
    refresh_shell(app);
    if !show {
        return Ok(());
    }
    // Locked, or with previews off, a notification only says which account has news.
    let (title, body) = if locked || !preview {
        (label, "New message".to_string())
    } else {
        (format!("{} · {}", short(title, 100), label), short(body, 300))
    };
    let click_app = app.clone();
    system::toast(&app.config().identifier, &title, &body, move || {
        open_message(&click_app, &account_id, id)
    })
}

fn on_panic_shortcut(app: &AppHandle) {
    let Some(window) = app.get_window("main") else { return };
    let showing = window.is_visible().unwrap_or(false) && !window.is_minimized().unwrap_or(false);
    if showing {
        lock_app(app);
        hide_pops(app);
        hide_window(app);
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
            open_recent,
            clear_recent,
            set_realtime,
            set_notify,
            toggle_pop,
            close_pop,
            page_state,
            account_action,
            reply_recent,
            dismiss_away,
            answer_update,
            horoscope,
            notify
        ])
        .on_menu_event(|app, event| menus::on_menu_event(app, event.id().as_ref()))
        .setup(|app| {
            let config_dir = app.path().app_config_dir()?;
            let accounts_file = config_dir.join("accounts.json");
            let settings_file = config_dir.join("settings.json");
            let data_dir = app.path().app_local_data_dir()?;
            let profiles = data_dir.join("profiles");
            // Notifications show the app's name and icon (see system::register_app_id).
            let icon = data_dir.join("notification-icon.png");
            if std::fs::create_dir_all(&data_dir).and_then(|_| std::fs::write(&icon, include_bytes!("../icons/128x128.png"))).is_ok() {
                if let Err(e) = system::register_app_id(&app.config().identifier, "Master Chat", &icon) {
                    eprintln!("could not register for notifications: {e}");
                }
            }
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
                    // The shell opens on the overview.
                    overlay: true,
                    unread: HashMap::new(),
                    locked: settings.lock.enabled,
                    failed_unlocks: 0,
                    unlock_retry_at: 0,
                    focused: true,
                    notify_log: HashMap::new(),
                    low_memory: HashMap::new(),
                    recent: VecDeque::new(),
                    loaded_at: HashMap::new(),
                    pops: Vec::new(),
                    pop_seq: 0,
                    signed_out: HashSet::new(),
                    history: HashMap::new(),
                    left_at: None,
                    away_since: None,
                    update: None,
                    offered: HashSet::new(),
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
            // All are created after setup: a webview made during setup never gets
            // later show/hide calls, so it would stay "visible" to its page (which
            // then holds back notifications and may mark messages as read).
            let (first, rest): (Vec<_>, Vec<_>) = accounts.into_iter().partition(|a| Some(&a.id) == active.as_ref());
            let (later_app, later_window, later_profiles) = (handle.clone(), window.clone(), profiles.clone());
            std::thread::spawn(move || {
                for (i, account) in first.into_iter().chain(rest).enumerate() {
                    if i > 0 {
                        std::thread::sleep(Duration::from_secs(if i == 1 { 3 } else { 4 }));
                    }
                    if let Err(e) = create_account_webview(&later_app, &later_window, &account, &later_profiles) {
                        eprintln!("could not open {}: {e}", account.label);
                    }
                    layout(&later_app);
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

            // Looks for updates a minute after starting, then every 6 hours.
            let update_app = handle.clone();
            std::thread::spawn(move || {
                std::thread::sleep(Duration::from_secs(60));
                loop {
                    tauri::async_runtime::block_on(look_for_update(&update_app));
                    std::thread::sleep(Duration::from_secs(6 * 3600));
                }
            });

            // Each account's unread count over the day, for the overview's charts.
            let history_app = handle.clone();
            std::thread::spawn(move || loop {
                let state = history_app.state::<AppState>();
                {
                    let mut inner = state.inner.lock().unwrap();
                    let counts: Vec<(String, u32)> =
                        inner.accounts.iter().map(|a| (a.id.clone(), inner.unread.get(&a.id).copied().unwrap_or(0))).collect();
                    inner.history.retain(|id, _| counts.iter().any(|(c, _)| c == id));
                    for (id, n) in counts {
                        let h = inner.history.entry(id).or_default();
                        h.push_back(n);
                        while h.len() > HISTORY_LEN {
                            h.pop_front();
                        }
                    }
                }
                refresh_shell(&history_app);
                std::thread::sleep(HISTORY_STEP);
            });

            // Locks the app after the set time without keyboard or mouse input.
            let idle_app = handle.clone();
            std::thread::spawn(move || loop {
                std::thread::sleep(Duration::from_secs(15));
                check_presence(&idle_app);
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
                    check_presence(&events_app);
                }
                WindowEvent::CloseRequested { api, .. } if events_app.state::<AppState>().settings.lock().unwrap().close_to_tray => {
                    api.prevent_close();
                    hide_window(&events_app);
                }
                _ => {}
            });
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running Master Chat");
}
