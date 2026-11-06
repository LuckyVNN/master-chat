use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Serialize, Deserialize, Clone)]
#[serde(default)]
pub struct Settings {
    /// "system", "light" or "dark".
    pub theme: String,
    pub show_labels: bool,
    pub close_to_tray: bool,
    pub start_with_windows: bool,
    pub start_minimized: bool,
    pub notifications: bool,
    /// Show the message text in notifications, not just who it's from.
    pub notification_preview: bool,
    /// Do Not Disturb until this time (seconds since 1970); u64::MAX = until turned off.
    pub dnd_until: Option<u64>,
    pub panic_shortcut: bool,
    pub lock: LockSettings,
}

#[derive(Serialize, Deserialize, Clone, Default)]
#[serde(default)]
pub struct LockSettings {
    pub enabled: bool,
    /// PBKDF2-SHA256 of the PIN, both hex.
    pub salt: String,
    pub hash: String,
    /// Lock after this many minutes without keyboard or mouse input; 0 = never.
    pub idle_minutes: u32,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            theme: "system".into(),
            show_labels: false,
            close_to_tray: true,
            start_with_windows: false,
            start_minimized: true,
            notifications: true,
            notification_preview: true,
            dnd_until: None,
            panic_shortcut: true,
            lock: LockSettings { idle_minutes: 10, ..Default::default() },
        }
    }
}

impl Settings {
    pub fn dnd_active(&self) -> bool {
        self.dnd_until.is_some_and(|until| until > crate::now_secs())
    }
}

pub fn load(path: &Path) -> Settings {
    let mut settings: Settings = std::fs::read_to_string(path)
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_default();
    if !matches!(settings.theme.as_str(), "system" | "light" | "dark" | "sakura") {
        settings.theme = "system".into();
    }
    settings
}

pub fn save(path: &Path, settings: &Settings) -> Result<(), String> {
    crate::accounts::write_json(path, settings)
}

/// What the shell may see: everything except the PIN's hash and salt.
#[derive(Serialize)]
pub struct SettingsView {
    pub theme: String,
    pub show_labels: bool,
    pub close_to_tray: bool,
    pub start_with_windows: bool,
    pub start_minimized: bool,
    pub notifications: bool,
    pub notification_preview: bool,
    pub dnd_until: Option<u64>,
    pub panic_shortcut: bool,
    pub lock_enabled: bool,
    pub lock_idle_minutes: u32,
}

impl From<&Settings> for SettingsView {
    fn from(s: &Settings) -> Self {
        SettingsView {
            theme: s.theme.clone(),
            show_labels: s.show_labels,
            close_to_tray: s.close_to_tray,
            start_with_windows: s.start_with_windows,
            start_minimized: s.start_minimized,
            notifications: s.notifications,
            notification_preview: s.notification_preview,
            dnd_until: s.dnd_until.filter(|_| s.dnd_active()),
            panic_shortcut: s.panic_shortcut,
            lock_enabled: s.lock.enabled,
            lock_idle_minutes: s.lock.idle_minutes,
        }
    }
}

/// The settings the shell can change in one go; the PIN has its own commands.
#[derive(Deserialize)]
pub struct SettingsUpdate {
    pub theme: Option<String>,
    pub show_labels: Option<bool>,
    pub close_to_tray: Option<bool>,
    pub start_with_windows: Option<bool>,
    pub start_minimized: Option<bool>,
    pub notifications: Option<bool>,
    pub notification_preview: Option<bool>,
    pub panic_shortcut: Option<bool>,
    pub lock_idle_minutes: Option<u32>,
}
