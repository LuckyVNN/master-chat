//! What the app needs from Windows itself: notifications, start with Windows,
//! and how long the computer has been idle.

#[cfg(windows)]
mod imp {
    use tauri_winrt_notification::Toast;
    use winreg::{enums::*, RegKey};

    const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
    const RUN_VALUE: &str = "Master Chat";

    pub fn set_autostart(enable: bool) -> Result<(), String> {
        let run = RegKey::predef(HKEY_CURRENT_USER)
            .open_subkey_with_flags(RUN_KEY, KEY_SET_VALUE)
            .map_err(|e| e.to_string())?;
        if enable {
            let exe = std::env::current_exe().map_err(|e| e.to_string())?;
            run.set_value(RUN_VALUE, &format!("\"{}\" --autostart", exe.display())).map_err(|e| e.to_string())
        } else {
            match run.delete_value(RUN_VALUE) {
                Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e.to_string()),
                _ => Ok(()),
            }
        }
    }

    pub fn idle_seconds() -> u64 {
        use windows_sys::Win32::System::SystemInformation::GetTickCount;
        use windows_sys::Win32::UI::Input::KeyboardAndMouse::{GetLastInputInfo, LASTINPUTINFO};
        let mut info = LASTINPUTINFO { cbSize: std::mem::size_of::<LASTINPUTINFO>() as u32, dwTime: 0 };
        // SAFETY: info is a valid LASTINPUTINFO with cbSize set, as the API requires.
        if unsafe { GetLastInputInfo(&mut info) } == 0 {
            return 0;
        }
        (unsafe { GetTickCount() }.wrapping_sub(info.dwTime) / 1000) as u64
    }

    pub fn toast(app_id: &str, title: &str, body: &str, on_click: impl Fn() + Send + 'static) -> Result<(), String> {
        Toast::new(app_id)
            .title(title)
            .text1(body)
            .on_activated(move |_| {
                on_click();
                Ok(())
            })
            .show()
            .map_err(|e| e.to_string())
    }

    /// The installer registers the app under its identifier; a dev build isn't
    /// installed, so its notifications show as coming from PowerShell.
    pub fn app_id(identifier: &str) -> String {
        if cfg!(debug_assertions) {
            Toast::POWERSHELL_APP_ID.to_string()
        } else {
            identifier.to_string()
        }
    }

    /// Asks WebView2 to keep a page that's off screen small: it keeps running
    /// (notifications, unread counts) but gives back memory it can spare.
    pub fn set_low_memory(webview: &tauri::Webview, low: bool) {
        use webview2_com::Microsoft::Web::WebView2::Win32::{
            ICoreWebView2_19, COREWEBVIEW2_MEMORY_USAGE_TARGET_LEVEL_LOW, COREWEBVIEW2_MEMORY_USAGE_TARGET_LEVEL_NORMAL,
        };
        use windows_core::Interface;
        let level = if low { COREWEBVIEW2_MEMORY_USAGE_TARGET_LEVEL_LOW } else { COREWEBVIEW2_MEMORY_USAGE_TARGET_LEVEL_NORMAL };
        let _ = webview.with_webview(move |platform| unsafe {
            if let Ok(core) = platform.controller().CoreWebView2().and_then(|c| c.cast::<ICoreWebView2_19>()) {
                let _ = core.SetMemoryUsageTargetLevel(level);
            }
        });
    }
}

#[cfg(not(windows))]
mod imp {
    pub fn set_autostart(_enable: bool) -> Result<(), String> {
        Err("only supported on Windows".into())
    }
    pub fn idle_seconds() -> u64 {
        0
    }
    pub fn toast(_app_id: &str, _title: &str, _body: &str, _on_click: impl Fn() + Send + 'static) -> Result<(), String> {
        Ok(())
    }
    pub fn app_id(identifier: &str) -> String {
        identifier.to_string()
    }
    pub fn set_low_memory(_webview: &tauri::Webview, _low: bool) {}
}

pub use imp::*;
