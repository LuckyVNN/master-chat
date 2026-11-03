use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Serialize, Deserialize, Clone)]
pub struct Account {
    pub id: String,
    pub service: String,
    pub label: String,
    /// Icon color in place of the brand color, "#rrggbb".
    #[serde(default)]
    pub color: Option<String>,
    #[serde(default = "one")]
    pub zoom: f64,
    #[serde(default = "yes")]
    pub notify: bool,
    /// Messenger: don't let people see that you're typing.
    #[serde(default)]
    pub block_typing: bool,
    /// Kept fully awake while hidden, so messages arrive and notify at once.
    /// Otherwise the page may idle (timers slowed to once a minute) to save
    /// memory and power.
    #[serde(default = "yes")]
    pub realtime: bool,
    /// Its unread count isn't shown or added to the total (an inbox nobody empties).
    #[serde(default)]
    pub hide_unread: bool,
}

fn one() -> f64 {
    1.0
}

fn yes() -> bool {
    true
}

impl Account {
    pub fn new(id: String, service: String, label: String) -> Self {
        // Gemini has no messages to wait for.
        let realtime = service != "gemini";
        Account { id, service, label, color: None, zoom: 1.0, notify: true, block_typing: false, realtime, hide_unread: false }
    }
}

pub fn load(path: &Path) -> Vec<Account> {
    let Ok(text) = std::fs::read_to_string(path) else { return Vec::new() };
    let accounts: Vec<Account> = serde_json::from_str(&text).unwrap_or_default();
    // Ids become folder names, so never trust a hand-edited file.
    accounts
        .into_iter()
        .filter(|a| valid_id(&a.id) && crate::services::find(&a.service).is_some())
        .map(|mut a| {
            a.zoom = clamp_zoom(a.zoom);
            a.color = a.color.filter(|c| valid_color(c));
            a
        })
        .collect()
}

pub fn save(path: &Path, accounts: &[Account]) -> Result<(), String> {
    write_json(path, accounts)
}

pub fn write_json<T: Serialize + ?Sized>(path: &Path, value: &T) -> Result<(), String> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    let text = serde_json::to_string_pretty(value).map_err(|e| e.to_string())?;
    // Write then rename, so a crash never leaves a half-written file.
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, text).map_err(|e| e.to_string())?;
    std::fs::rename(&tmp, path).map_err(|e| e.to_string())
}

pub fn new_id() -> String {
    static COUNTER: AtomicU32 = AtomicU32::new(0);
    let nanos = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_nanos());
    format!("{:x}{:04x}", nanos, COUNTER.fetch_add(1, Ordering::Relaxed) & 0xffff)
}

pub fn valid_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= 40 && id.bytes().all(|b| b.is_ascii_hexdigit())
}

pub fn valid_color(color: &str) -> bool {
    color.len() == 7 && color.starts_with('#') && color[1..].bytes().all(|b| b.is_ascii_hexdigit())
}

pub fn clamp_zoom(zoom: f64) -> f64 {
    if zoom.is_finite() {
        (zoom * 10.0).round().clamp(5.0, 30.0) / 10.0
    } else {
        1.0
    }
}

pub fn clean_label(label: &str) -> String {
    label.chars().filter(|c| !c.is_control()).take(40).collect::<String>().trim().to_string()
}

pub fn profile_dir(profiles: &Path, id: &str) -> PathBuf {
    profiles.join(id)
}
