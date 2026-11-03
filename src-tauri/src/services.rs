use serde::Serialize;
use tauri::Url;

#[derive(Serialize, Clone, Copy)]
pub struct Service {
    pub id: &'static str,
    pub name: &'static str,
    pub url: &'static str,
    pub color: &'static str,
    /// Hosts the service's webview may navigate to (sign-in pages included);
    /// subdomains are allowed too. Anything else opens in the system browser.
    #[serde(skip)]
    pub domains: &'static [&'static str],
    /// When set, new windows (target=_blank, window.open) stay in the app only for
    /// these paths; every other link opens in the system browser.
    #[serde(skip)]
    pub popup_paths: Option<&'static [&'static str]>,
    /// Script run at document start in the service's main frame, to restyle it.
    #[serde(skip)]
    pub inject: Option<&'static str>,
}

pub const SERVICES: &[Service] = &[
    Service {
        id: "gmail",
        name: "Gmail",
        url: "https://mail.google.com/mail/",
        color: "#ea4335",
        domains: crate::google_domains::GMAIL_DOMAINS,
        popup_paths: None,
        inject: None,
    },
    Service {
        id: "outlook",
        name: "Outlook",
        url: "https://outlook.live.com/mail/",
        color: "#0078d4",
        domains: &[
            "live.com",
            "outlook.com",
            "office.com",
            "office365.com",
            "cloud.microsoft",
            "microsoft.com",
            "microsoftonline.com",
            "msauth.net",
            "msftauth.net",
        ],
        popup_paths: None,
        inject: Some(include_str!("inject/outlook.js")),
    },
    Service {
        id: "messenger",
        name: "Messenger",
        url: "https://www.facebook.com/messages/",
        color: "#a033ff",
        domains: &["facebook.com", "messenger.com", "fbcdn.net", "fbsbx.com"],
        popup_paths: Some(&["/groupcall", "/videocall", "/messenger_media"]),
        inject: Some(include_str!("inject/messenger.js")),
    },
    Service {
        id: "zalo",
        name: "Zalo",
        url: "https://chat.zalo.me/",
        color: "#0068ff",
        domains: &["zalo.me", "zaloapp.com", "zdn.vn"],
        popup_paths: None,
        inject: None,
    },
    Service {
        id: "whatsapp",
        name: "WhatsApp",
        url: "https://web.whatsapp.com/",
        color: "#25d366",
        domains: &["whatsapp.com", "whatsapp.net"],
        popup_paths: None,
        inject: None,
    },
    Service {
        id: "telegram",
        name: "Telegram",
        url: "https://web.telegram.org/a/",
        color: "#29a9eb",
        domains: &["telegram.org", "t.me"],
        popup_paths: None,
        inject: None,
    },
    Service {
        id: "discord",
        name: "Discord",
        url: "https://discord.com/app",
        color: "#5865f2",
        domains: &["discord.com", "discordapp.com", "discord.gg", "hcaptcha.com"],
        popup_paths: None,
        inject: None,
    },
];

pub fn find(id: &str) -> Option<&'static Service> {
    SERVICES.iter().find(|s| s.id == id)
}

impl Service {
    /// Only HTTPS pages on the service's own domains may load in its webview.
    pub fn allows(&self, url: &Url) -> bool {
        if url.as_str() == "about:blank" {
            return true;
        }
        if url.scheme() != "https" {
            return false;
        }
        let Some(host) = url.host_str() else { return false };
        self.domains
            .iter()
            .any(|d| host == *d || host.strip_suffix(d).is_some_and(|rest| rest.ends_with('.')))
    }
}

impl Service {
    /// New windows stay in the app only on the service's domains and, if it
    /// limits them, on its popup paths (calls, the photo viewer).
    pub fn allows_popup(&self, url: &Url) -> bool {
        self.allows(url) && self.popup_paths.is_none_or(|paths| paths.iter().any(|p| url.path().starts_with(p)))
    }
}

/// Unread count from a page title such as "(3) WhatsApp" or "Inbox (12) - Gmail".
pub fn unread_from_title(title: &str) -> u32 {
    let mut rest = title;
    while let Some(start) = rest.find('(') {
        rest = &rest[start + 1..];
        if let Some(end) = rest.find(')') {
            let inner = rest[..end].trim().trim_end_matches('+');
            if let Ok(n) = inner.replace(',', "").parse::<u32>() {
                return n;
            }
        }
    }
    0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn allowlist() {
        let gmail = find("gmail").unwrap();
        assert!(gmail.allows(&Url::parse("https://mail.google.com/mail/u/0").unwrap()));
        assert!(gmail.allows(&Url::parse("https://accounts.google.com/").unwrap()));
        assert!(gmail.allows(&Url::parse("https://accounts.google.com.vn/accounts/SetSID").unwrap()));
        assert!(!gmail.allows(&Url::parse("https://google.com.evil.vn/").unwrap()));
        assert!(!gmail.allows(&Url::parse("http://mail.google.com/").unwrap()));
        assert!(!gmail.allows(&Url::parse("https://evilgoogle.com/").unwrap()));
        assert!(!gmail.allows(&Url::parse("https://google.com.evil.net/").unwrap()));
        assert!(!gmail.allows(&Url::parse("file:///C:/Windows").unwrap()));

        let messenger = find("messenger").unwrap();
        assert!(messenger.allows_popup(&Url::parse("https://www.facebook.com/groupcall/ROOM:123").unwrap()));
        assert!(!messenger.allows_popup(&Url::parse("https://www.facebook.com/reel/123").unwrap()));
        assert!(gmail.allows_popup(&Url::parse("https://accounts.google.com/").unwrap()));
    }

    #[test]
    fn titles() {
        assert_eq!(unread_from_title("(3) WhatsApp"), 3);
        assert_eq!(unread_from_title("Inbox (1,204) - me@gmail.com - Gmail"), 1204);
        assert_eq!(unread_from_title("Discord | (beta) (5)"), 5);
        assert_eq!(unread_from_title("Telegram"), 0);
    }
}
