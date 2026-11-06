//! Oha Asa's daily horoscope (Asahi TV's morning show): the twelve signs in
//! today's order, from the data file its web page reads, translated to English.
//! Fetched by the app, not the sidebar page, which may load nothing from the web.

use serde::{Deserialize, Serialize};
use std::sync::Mutex;
use std::time::Duration;

const SOURCE: &str = "https://www.asahi.co.jp/data/ohaasa2020/horoscope.json";
const TRANSLATE: &str = "https://translate.googleapis.com/translate_a/single?client=gtx&sl=ja&tl=en&dt=t";
/// The show updates on weekday mornings; a fresh copy at most this often.
const KEEP_SECS: u64 = 3600;

/// The show's sign codes, in its order.
const SIGNS: [(&str, &str, &str); 12] = [
    ("01", "Aries", "おひつじ座"),
    ("02", "Taurus", "おうし座"),
    ("03", "Gemini", "ふたご座"),
    ("04", "Cancer", "かに座"),
    ("05", "Leo", "しし座"),
    ("06", "Virgo", "おとめ座"),
    ("07", "Libra", "てんびん座"),
    ("08", "Scorpio", "さそり座"),
    ("09", "Sagittarius", "いて座"),
    ("10", "Capricorn", "やぎ座"),
    ("11", "Aquarius", "みずがめ座"),
    ("12", "Pisces", "うお座"),
];

#[derive(Serialize, Clone)]
pub struct Sign {
    rank: u8,
    name: String,
    name_ja: String,
    lines: Vec<String>,
    lines_ja: Vec<String>,
    lucky: String,
    lucky_ja: String,
}

#[derive(Serialize, Clone)]
pub struct Horoscope {
    /// The broadcast day, YYYY-MM-DD.
    date: String,
    signs: Vec<Sign>,
    /// False if the translation service couldn't be reached: the text is Japanese.
    translated: bool,
}

#[derive(Deserialize)]
struct Day {
    onair_date: String,
    detail: Vec<Detail>,
}

#[derive(Deserialize)]
struct Detail {
    ranking_no: String,
    horoscope_st: String,
    horoscope_text: String,
}

static CACHE: Mutex<Option<(u64, Horoscope)>> = Mutex::new(None);

fn client() -> Result<reqwest::Client, String> {
    if rustls::crypto::CryptoProvider::get_default().is_none() {
        let _ = rustls::crypto::ring::default_provider().install_default();
    }
    reqwest::Client::builder()
        .timeout(Duration::from_secs(15))
        // The translation service turns away requests that don't look like a browser's.
        .user_agent("Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/130.0 Safari/537.36")
        .build()
        .map_err(|e| e.to_string())
}

/// Today's horoscope: kept for an hour, and the translation for the whole day.
pub async fn get(refresh: bool) -> Result<Horoscope, String> {
    let cached = CACHE.lock().unwrap().clone();
    if let Some((at, ref h)) = cached {
        if !refresh && crate::now_secs().saturating_sub(at) < KEEP_SECS {
            return Ok(h.clone());
        }
    }
    let client = client()?;
    let text = client.get(SOURCE).send().await.map_err(|e| e.to_string())?.text().await.map_err(|e| e.to_string())?;
    let days: Vec<Day> = serde_json::from_str(&text).map_err(|e| e.to_string())?;
    let day = days.into_iter().next().ok_or("No horoscope has been published yet.")?;
    let raw = &day.onair_date;
    let date = if raw.len() == 8 { format!("{}-{}-{}", &raw[..4], &raw[4..6], &raw[6..]) } else { raw.clone() };

    // The same day, already translated: nothing to do again.
    if let Some((_, h)) = cached.filter(|(_, h)| h.date == date && h.translated) {
        *CACHE.lock().unwrap() = Some((crate::now_secs(), h.clone()));
        return Ok(h);
    }

    let mut signs = Vec::new();
    let mut translated = true;
    for d in day.detail {
        let Some(&(_, name, name_ja)) = SIGNS.iter().find(|(code, _, _)| *code == d.horoscope_st) else { continue };
        // Tab-separated: up to four lines (some empty), then the lucky item last.
        let mut fields: Vec<String> = d.horoscope_text.split('\t').map(|f| f.trim().to_string()).collect();
        let lucky_ja = fields.pop().unwrap_or_default();
        let lines_ja: Vec<String> = fields.into_iter().filter(|f| !f.is_empty()).collect();
        let mut lines = lines_ja.clone();
        let mut lucky = lucky_ja.clone();
        if translated {
            let source = lines_ja.iter().chain(std::iter::once(&lucky_ja)).cloned().collect::<Vec<_>>().join("\n");
            match translate(&client, &source).await {
                Some(english) => {
                    let mut parts: Vec<String> = english.split('\n').map(|s| s.trim().to_string()).collect();
                    if parts.len() == lines_ja.len() + 1 {
                        lucky = capitalize(&parts.pop().unwrap_or_default());
                        lines = parts;
                    }
                }
                None => translated = false,
            }
        }
        signs.push(Sign {
            rank: d.ranking_no.parse().unwrap_or(0),
            name: name.into(),
            name_ja: name_ja.into(),
            lines,
            lines_ja,
            lucky,
            lucky_ja,
        });
    }
    if signs.is_empty() {
        return Err("No horoscope has been published yet.".into());
    }
    signs.sort_by_key(|s| s.rank);
    if !translated {
        for s in &mut signs {
            s.lines = s.lines_ja.clone();
            s.lucky = s.lucky_ja.clone();
        }
    }
    let horoscope = Horoscope { date, signs, translated };
    *CACHE.lock().unwrap() = Some((crate::now_secs(), horoscope.clone()));
    Ok(horoscope)
}

async fn translate(client: &reqwest::Client, text: &str) -> Option<String> {
    let response = client.post(TRANSLATE).form(&[("q", text)]).send().await.ok()?;
    if !response.status().is_success() {
        return None;
    }
    let json: serde_json::Value = serde_json::from_str(&response.text().await.ok()?).ok()?;
    let pieces = json.get(0)?.as_array()?;
    Some(pieces.iter().filter_map(|p| p.get(0)?.as_str()).collect())
}

fn capitalize(s: &str) -> String {
    let mut chars = s.chars();
    chars.next().map_or_else(String::new, |c| c.to_uppercase().chain(chars).collect())
}

#[cfg(test)]
mod tests {
    /// Reaches the network; run with `cargo test -- --ignored`.
    #[test]
    #[ignore]
    fn live() {
        let h = tauri::async_runtime::block_on(super::get(true)).unwrap();
        assert_eq!(h.signs.len(), 12);
        for s in &h.signs {
            println!("{} {} — {} | lucky: {}", s.rank, s.name, s.lines.join(" / "), s.lucky);
        }
        assert!(h.translated);
    }
}
