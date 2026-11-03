use tauri::Url;

/// Query parameters that only exist to track who clicked what.
const TRACKING_PARAMS: &[&str] = &[
    "fbclid", "gclid", "dclid", "gbraid", "wbraid", "msclkid", "igshid", "mc_eid", "mc_cid", "yclid", "_hsenc",
    "_hsmi", "__cft__", "__tn__", "mkt_tok", "si", "ref_src",
];

/// The link as it should open in the browser: out of Facebook's and Google's
/// redirect pages, and without tracking parameters.
pub fn clean(url: &Url) -> Url {
    let mut url = unwrap_redirect(url).unwrap_or_else(|| url.clone());
    let kept: Vec<(String, String)> = url
        .query_pairs()
        .filter(|(k, _)| !k.starts_with("utm_") && !TRACKING_PARAMS.contains(&k.as_ref()))
        .map(|(k, v)| (k.into_owned(), v.into_owned()))
        .collect();
    if url.query().is_some() {
        if kept.is_empty() {
            url.set_query(None);
        } else {
            url.query_pairs_mut().clear().extend_pairs(kept);
        }
    }
    url
}

fn unwrap_redirect(url: &Url) -> Option<Url> {
    let host = url.host_str()?;
    let param = match (host, url.path()) {
        ("l.facebook.com" | "lm.facebook.com" | "l.messenger.com" | "l.instagram.com", "/l.php" | "/") => "u",
        (h, "/url") if h == "google.com" || h.ends_with(".google.com") => "q",
        _ => return None,
    };
    let target = url.query_pairs().find(|(k, _)| k == param || (param == "q" && k == "url"))?.1;
    let target = Url::parse(&target).ok()?;
    matches!(target.scheme(), "http" | "https").then_some(target)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cleaned(s: &str) -> String {
        clean(&Url::parse(s).unwrap()).to_string()
    }

    #[test]
    fn unwraps_and_strips() {
        assert_eq!(
            cleaned("https://l.facebook.com/l.php?u=https%3A%2F%2Fexample.com%2Fa%3Fx%3D1%26fbclid%3Dabc&h=AT0"),
            "https://example.com/a?x=1"
        );
        assert_eq!(cleaned("https://www.google.com/url?q=https://example.com/&sa=D"), "https://example.com/");
        assert_eq!(cleaned("https://example.com/?utm_source=x&utm_medium=y"), "https://example.com/");
        assert_eq!(cleaned("https://example.com/?id=5&gclid=1"), "https://example.com/?id=5");
        // A redirect to something other than a web page is left alone.
        assert_eq!(
            cleaned("https://l.facebook.com/l.php?u=javascript%3Aalert(1)"),
            "https://l.facebook.com/l.php?u=javascript%3Aalert%281%29"
        );
    }
}
