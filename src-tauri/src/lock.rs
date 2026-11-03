//! The app lock's PIN: stored only as a salted PBKDF2-SHA256 hash.

use sha2::Sha256;

const ROUNDS: u32 = 300_000;

pub fn valid_pin(pin: &str) -> bool {
    (4..=32).contains(&pin.chars().count())
}

/// A new random salt and the PIN's hash, both hex.
pub fn hash_new(pin: &str) -> Result<(String, String), String> {
    let mut salt = [0u8; 16];
    getrandom::fill(&mut salt).map_err(|e| e.to_string())?;
    Ok((hex(&salt), hex(&derive(pin, &salt))))
}

pub fn verify(pin: &str, salt_hex: &str, hash_hex: &str) -> bool {
    let (Some(salt), Some(hash)) = (unhex(salt_hex), unhex(hash_hex)) else { return false };
    let derived = derive(pin, &salt);
    // Compare every byte, so the time taken says nothing about the PIN.
    derived.len() == hash.len() && derived.iter().zip(&hash).fold(0u8, |acc, (a, b)| acc | (a ^ b)) == 0
}

fn derive(pin: &str, salt: &[u8]) -> [u8; 32] {
    let mut out = [0u8; 32];
    pbkdf2::pbkdf2_hmac::<Sha256>(pin.as_bytes(), salt, ROUNDS, &mut out);
    out
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn unhex(s: &str) -> Option<Vec<u8>> {
    if s.len() % 2 != 0 {
        return None;
    }
    (0..s.len()).step_by(2).map(|i| u8::from_str_radix(&s[i..i + 2], 16).ok()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pin_roundtrip() {
        let (salt, hash) = hash_new("2468").unwrap();
        assert!(verify("2468", &salt, &hash));
        assert!(!verify("2469", &salt, &hash));
        assert!(!verify("2468", "zz", &hash));
    }
}
