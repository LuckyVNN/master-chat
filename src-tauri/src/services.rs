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
    /// The service's sign-in pages (exact hosts). A school or work account may be
    /// sent from them to its organization's own login page on any domain; that
    /// page may load in the app until sign-in comes back to the service.
    #[serde(skip)]
    pub sign_in_hosts: &'static [&'static str],
    /// When set, new windows (target=_blank, window.open) stay in the app only for
    /// these paths; every other link opens in the system browser.
    #[serde(skip)]
    pub popup_paths: Option<&'static [&'static str]>,
    /// Script run at document start in the service's main frame, to restyle it.
    #[serde(skip)]
    pub inject: Option<&'static str>,
    /// Its script raises notifications itself, describing each new message, so
    /// the app adds no plain notice when the unread count rises.
    #[serde(skip)]
    pub describes_messages: bool,
    /// Path prefixes of a single conversation's page, which can be popped out
    /// into a window of its own. Empty where a chat has no address of its own,
    /// or the service allows only one open copy (WhatsApp).
    #[serde(skip)]
    pub chat_paths: &'static [&'static str],
}

pub const SERVICES: &[Service] = &[
    Service {
        id: "gmail",
        name: "Gmail",
        url: "https://mail.google.com/mail/",
        color: "#ea4335",
        domains: crate::google_domains::GOOGLE_DOMAINS,
        sign_in_hosts: &["accounts.google.com"],
        popup_paths: None,
        inject: Some(include_str!("inject/gmail.js")),
        describes_messages: true,
        chat_paths: &[],
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
        sign_in_hosts: &["login.microsoftonline.com", "login.live.com"],
        popup_paths: None,
        inject: Some(include_str!("inject/outlook.js")),
        describes_messages: false,
        chat_paths: &[],
    },
    Service {
        id: "messenger",
        name: "Messenger",
        url: "https://www.facebook.com/messages/",
        color: "#a033ff",
        domains: &["facebook.com", "messenger.com", "fbcdn.net", "fbsbx.com"],
        sign_in_hosts: &[],
        popup_paths: Some(&["/groupcall", "/videocall", "/messenger_media"]),
        inject: Some(include_str!("inject/messenger.js")),
        describes_messages: true,
        chat_paths: &["/messages/t/", "/messages/e2ee/t/"],
    },
    Service {
        id: "zalo",
        name: "Zalo",
        url: "https://chat.zalo.me/",
        color: "#0068ff",
        domains: &["zalo.me", "zaloapp.com", "zdn.vn"],
        sign_in_hosts: &[],
        popup_paths: None,
        inject: None,
        describes_messages: false,
        chat_paths: &[],
    },
    Service {
        id: "whatsapp",
        name: "WhatsApp",
        url: "https://web.whatsapp.com/",
        color: "#25d366",
        domains: &["whatsapp.com", "whatsapp.net"],
        sign_in_hosts: &[],
        popup_paths: None,
        inject: None,
        describes_messages: false,
        chat_paths: &[],
    },
    Service {
        id: "telegram",
        name: "Telegram",
        url: "https://web.telegram.org/a/",
        color: "#29a9eb",
        domains: &["telegram.org", "t.me"],
        sign_in_hosts: &[],
        popup_paths: None,
        inject: None,
        describes_messages: false,
        chat_paths: &[],
    },
    Service {
        id: "discord",
        name: "Discord",
        url: "https://discord.com/app",
        color: "#5865f2",
        domains: &["discord.com", "discordapp.com", "discord.gg", "hcaptcha.com"],
        sign_in_hosts: &[],
        popup_paths: None,
        inject: None,
        describes_messages: false,
        chat_paths: &["/channels/"],
    },
    Service {
        id: "gemini",
        name: "Gemini",
        url: "https://gemini.google.com/app",
        color: "#8e75b2",
        domains: crate::google_domains::GOOGLE_DOMAINS,
        sign_in_hosts: &["accounts.google.com"],
        // Sources and search results it links to open in the browser.
        popup_paths: Some(&[]),
        inject: None,
        describes_messages: false,
        chat_paths: &[],
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
    /// A single conversation's page, which can be popped out.
    pub fn is_chat(&self, url: &Url) -> bool {
        self.allows(url) && self.chat_paths.iter().any(|p| url.path().len() > p.len() && url.path().starts_with(p))
    }

    pub fn is_sign_in(&self, url: &Url) -> bool {
        url.scheme() == "https" && url.host_str().is_some_and(|host| self.sign_in_hosts.contains(&host))
    }

    /// New windows stay in the app only on the service's domains and, if it
    /// limits them, on its popup paths (calls, the photo viewer).
    pub fn allows_popup(&self, url: &Url) -> bool {
        // A call window opens blank first, then loads the call.
        url.as_str() == "about:blank"
            || self.allows(url) && self.popup_paths.is_none_or(|paths| paths.iter().any(|p| url.path().starts_with(p)))
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
        assert!(messenger.allows_popup(&Url::parse("about:blank").unwrap()));
        assert!(gmail.allows_popup(&Url::parse("https://accounts.google.com/").unwrap()));
        assert!(gmail.is_sign_in(&Url::parse("https://accounts.google.com/v3/signin").unwrap()));
        assert!(!gmail.is_sign_in(&Url::parse("https://mail.google.com/").unwrap()));
        assert!(!messenger.is_sign_in(&Url::parse("https://www.facebook.com/login").unwrap()));
        assert!(messenger.is_chat(&Url::parse("https://www.facebook.com/messages/t/1234").unwrap()));
        assert!(messenger.is_chat(&Url::parse("https://www.facebook.com/messages/e2ee/t/1234").unwrap()));
        assert!(!messenger.is_chat(&Url::parse("https://www.facebook.com/messages/t/").unwrap()));
        assert!(!messenger.is_chat(&Url::parse("https://www.facebook.com/messages/").unwrap()));
        assert!(!gmail.is_chat(&Url::parse("https://mail.google.com/mail/u/0/#inbox").unwrap()));
    }

    #[test]
    fn titles() {
        assert_eq!(unread_from_title("(3) WhatsApp"), 3);
        assert_eq!(unread_from_title("Inbox (1,204) - me@gmail.com - Gmail"), 1204);
        assert_eq!(unread_from_title("Discord | (beta) (5)"), 5);
        assert_eq!(unread_from_title("Telegram"), 0);
    }
}
