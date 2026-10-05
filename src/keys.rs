use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::time::{SystemTime, UNIX_EPOCH};

/// A jengine access key as stored: only its SHA-256 is kept, the key itself is shown once at creation.
#[derive(Clone, Serialize, Deserialize)]
pub struct StoredKey {
    pub id: String,
    pub name: String,
    /// The first characters of the key, enough to recognise it in a list.
    pub prefix: String,
    pub sha256: String,
    pub created_at: u64,
    /// Unix seconds; `None` never expires.
    pub expires_at: Option<u64>,
}

impl StoredKey {
    pub fn expired(&self, now: u64) -> bool {
        self.expires_at.is_some_and(|t| t <= now)
    }
}

pub fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn random_hex(len: usize) -> String {
    let mut bytes = vec![0u8; len];
    getrandom::fill(&mut bytes).expect("the OS random generator is unavailable");
    hex(&bytes)
}

fn sha256(text: &str) -> String {
    hex(&Sha256::digest(text.as_bytes()))
}

/// Compares in constant time, so response timing does not reveal how much of a secret matched.
fn same(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).fold(0, |acc, (x, y)| acc | (x ^ y)) == 0
}

/// Creates a key with 256 bits from the OS random generator. Returns the key to show once and the record to store.
pub fn generate(name: &str, expires_at: Option<u64>) -> (String, StoredKey) {
    let key = format!("jgn_{}", random_hex(32));
    let record = StoredKey {
        id: random_hex(4),
        name: name.to_owned(),
        prefix: key[..12].to_owned(),
        sha256: sha256(&key),
        created_at: now(),
        expires_at,
    };
    (key, record)
}

/// Whether the `Authorization` header carries a static key or a stored key that has not expired.
pub fn verify(
    header: Option<&str>,
    static_keys: &[String],
    stored: &[StoredKey],
    now: u64,
) -> bool {
    let Some(given) = header.and_then(|h| h.strip_prefix("Bearer ")) else {
        return false;
    };
    let digest = sha256(given);
    // Every key is checked without stopping at the first match, so timing does not reveal which one matched.
    let static_ok = static_keys
        .iter()
        .fold(false, |ok, k| ok | same(given.as_bytes(), k.as_bytes()));
    let stored_ok = stored.iter().fold(false, |ok, k| {
        ok | (same(digest.as_bytes(), k.sha256.as_bytes()) & !k.expired(now))
    });
    static_ok | stored_ok
}

/// Days since 1970-01-01 for a proleptic Gregorian date.
fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let doy = (153 * (m + if m > 2 { -3 } else { 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// `never`, a duration such as `30d`, or a date `YYYY-MM-DD` (the key works through the end of that day, UTC).
pub fn parse_expiry(s: &str, now: u64) -> Result<Option<u64>, String> {
    let s = s.trim();
    if s.eq_ignore_ascii_case("never") {
        return Ok(None);
    }
    let at = if let Some(days) = s.strip_suffix('d') {
        let days: u64 = days
            .parse()
            .map_err(|_| format!("'{s}': expected a number of days such as 30d"))?;
        now + days * 86_400
    } else {
        let parts: Vec<i64> = s.split('-').map(|p| p.parse().unwrap_or(-1)).collect();
        let [y, m, d] = parts[..] else {
            return Err(format!("'{s}': expected never, 30d or YYYY-MM-DD"));
        };
        if !(1..=12).contains(&m) || !(1..=31).contains(&d) || y < 1970 {
            return Err(format!("'{s}' is not a valid date"));
        }
        (days_from_civil(y, m, d) + 1) as u64 * 86_400 - 1
    };
    if at <= now {
        return Err(format!("'{s}' is already in the past"));
    }
    Ok(Some(at))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn verifies_static_and_stored_keys() {
        let (key, mut record) = generate("ci", None);
        assert!(key.starts_with("jgn_") && key.len() == 68);
        let header = format!("Bearer {key}");
        let now = now();
        assert!(verify(Some(&header), &[], &[record.clone()], now));
        assert!(!verify(
            Some("Bearer jgn_wrong"),
            &[],
            &[record.clone()],
            now
        ));
        assert!(!verify(Some(&key), &[], &[record.clone()], now));
        record.expires_at = Some(now);
        assert!(!verify(Some(&header), &[], &[record], now));
        assert!(verify(Some("Bearer s3cret"), &["s3cret".into()], &[], now));
    }

    #[test]
    fn parses_expiry() {
        let now = 1_759_622_400; // 2025-10-05T00:00:00Z
        assert_eq!(parse_expiry("never", now).unwrap(), None);
        assert_eq!(parse_expiry("30d", now).unwrap(), Some(now + 30 * 86_400));
        assert_eq!(parse_expiry("2025-10-05", now).unwrap(), Some(now + 86_399));
        assert!(parse_expiry("2025-10-04", now).is_err());
        assert!(parse_expiry("2025-13-01", now).is_err());
        assert!(parse_expiry("soon", now).is_err());
    }
}
