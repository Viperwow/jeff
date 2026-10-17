use crate::{keys, log, modified, write_private};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

/// How long an admin page sign-in lasts; the cookie carries the same `Max-Age`.
pub const LIFETIME_SECS: u64 = 30 * 24 * 60 * 60;

#[derive(Clone, Serialize, Deserialize)]
pub struct Session {
    /// The SHA-256 of the key the session stands for, as the config stores it, so revoking the key ends the session.
    pub key_sha256: String,
    pub created_at: u64,
    pub expires_at: u64,
}

/// Admin page sign-ins, kept in `sessions.json` next to the config so a restart keeps them. A session has two halves:
/// one rides in an HttpOnly cookie, the other in a header the page sends. Browsers send a cookie to every port of the
/// host, so another local server can see the cookie half, but never the header half. Only the SHA-256 of both halves
/// is stored.
pub struct Sessions {
    path: PathBuf,
    config_path: String,
    by_digest: HashMap<String, Session>,
    /// When the file was last read, so a session removed by hand ends without a restart.
    mtime: Option<SystemTime>,
    /// Why the file failed to reload. The sessions read before stay in force, and jeff refuses to write over the file.
    problem: Option<String>,
}

pub fn path(config_path: &str) -> PathBuf {
    Path::new(config_path).with_file_name("sessions.json")
}

fn digest(cookie: &str, header: &str) -> String {
    keys::sha256(&format!("{cookie}.{header}"))
}

impl Sessions {
    pub fn new(config_path: &str) -> Self {
        let mut sessions = Sessions {
            path: path(config_path),
            config_path: config_path.to_owned(),
            by_digest: HashMap::new(),
            mtime: None,
            problem: None,
        };
        sessions.refresh();
        sessions
    }

    fn refresh(&mut self) {
        let now = modified(&self.path.to_string_lossy());
        if now == self.mtime && self.mtime.is_some() {
            return;
        }
        self.mtime = now;
        let read = match fs::read_to_string(&self.path) {
            Ok(text) => serde_json::from_str(&text).map_err(|e| e.to_string()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(HashMap::new()),
            Err(e) => Err(e.to_string()),
        };
        let file = self.path.display();
        match read {
            Ok(by_digest) => {
                self.by_digest = by_digest;
                if self.problem.take().is_some() {
                    let line = format!("{file} is fixed. jeff reloaded the sessions.");
                    eprintln!("{line}");
                    log::write(&self.config_path, "sessions_fixed", &line, &[]);
                }
            }
            Err(e) => {
                let line = format!(
                    "{file}: {e}. jeff keeps the sessions read before. Fix or delete the file."
                );
                eprintln!("error[sessions_invalid]: {line}");
                log::write(&self.config_path, "sessions_invalid", &line, &[]);
                self.problem = Some(format!("{file}: {e}"));
            }
        }
    }

    /// The key SHA-256 of the unexpired session both halves name.
    pub fn find(&mut self, cookie: &str, header: &str, now: u64) -> Option<String> {
        self.refresh();
        self.by_digest
            .get(&digest(cookie, header))
            .filter(|s| s.expires_at > now)
            .map(|s| s.key_sha256.clone())
    }

    /// Returns the cookie half and the header half. Expired sessions and sessions of keys `known` does not hold are
    /// dropped on the way.
    pub fn create(
        &mut self,
        key_sha256: &str,
        now: u64,
        known: impl Fn(&str) -> bool,
    ) -> Result<(String, String), String> {
        self.refresh();
        let before = self.by_digest.clone();
        let (cookie, header) = (keys::random_hex(32), keys::random_hex(32));
        self.by_digest.insert(
            digest(&cookie, &header),
            Session {
                key_sha256: key_sha256.to_owned(),
                created_at: now,
                expires_at: now + LIFETIME_SECS,
            },
        );
        self.by_digest
            .retain(|_, s| s.expires_at > now && known(&s.key_sha256));
        self.save(before)?;
        Ok((cookie, header))
    }

    pub fn remove(&mut self, cookie: &str, header: &str) -> Result<(), String> {
        self.refresh();
        let before = self.by_digest.clone();
        if self.by_digest.remove(&digest(cookie, header)).is_none() {
            return Ok(());
        }
        self.save(before)
    }

    /// Writes the sessions, or restores `before` and logs the cause, so memory never holds what the file lacks.
    fn save(&mut self, before: HashMap<String, Session>) -> Result<(), String> {
        let path = self.path.to_string_lossy().into_owned();
        let written = match &self.problem {
            Some(problem) => Err(format!("{problem}. Fix or delete the file.")),
            None => write_private(
                &path,
                &serde_json::to_string_pretty(&self.by_digest).unwrap(),
            ),
        };
        if let Err(e) = written {
            self.by_digest = before;
            eprintln!("error[sessions_not_saved]: {e}");
            log::write(&self.config_path, "sessions_not_saved", &e, &[]);
            return Err(e);
        }
        self.mtime = modified(&path);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(name: &str) -> String {
        let dir = std::env::temp_dir().join(format!("jeff-sessions-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir.join("config.json").to_string_lossy().into_owned()
    }

    #[test]
    fn a_session_needs_both_halves_and_survives_a_restart() {
        let config = temp("halves");
        let mut s = Sessions::new(&config);
        let (cookie, header) = s.create("k1", 100, |_| true).unwrap();
        assert_eq!(s.find(&cookie, &header, 101).as_deref(), Some("k1"));
        assert_eq!(s.find(&cookie, "", 101), None);
        assert_eq!(s.find("", &header, 101), None);
        assert_eq!(s.find(&cookie, &header, 100 + LIFETIME_SECS), None);

        let text = fs::read_to_string(path(&config)).unwrap();
        assert!(!text.contains(&cookie) && !text.contains(&header));
        let mut restarted = Sessions::new(&config);
        assert_eq!(restarted.find(&cookie, &header, 101).as_deref(), Some("k1"));

        restarted.remove(&cookie, &header).unwrap();
        assert_eq!(Sessions::new(&config).find(&cookie, &header, 101), None);
    }

    #[test]
    fn a_hand_edit_applies_and_a_broken_file_is_not_written_over() {
        let config = temp("edit");
        let mut s = Sessions::new(&config);
        let (cookie, header) = s.create("k1", 100, |_| true).unwrap();
        // The edit has to be seen as newer even where file times are coarse.
        s.mtime = Some(SystemTime::UNIX_EPOCH);
        fs::write(path(&config), "{}").unwrap();
        assert_eq!(s.find(&cookie, &header, 101), None);

        s.mtime = Some(SystemTime::UNIX_EPOCH);
        fs::write(path(&config), "{ broken").unwrap();
        assert_eq!(s.find(&cookie, &header, 101), None);
        assert!(s.create("k1", 101, |_| true).is_err());
        assert_eq!(fs::read_to_string(path(&config)).unwrap(), "{ broken");
    }

    #[test]
    fn a_failed_save_keeps_memory_as_the_file_is() {
        let config = temp("rollback");
        let mut s = Sessions::new(&config);
        let (cookie, header) = s.create("k1", 100, |_| true).unwrap();
        s.mtime = Some(SystemTime::UNIX_EPOCH);
        let text = fs::read_to_string(path(&config)).unwrap();
        fs::write(path(&config), format!("{text} broken")).unwrap();
        assert!(s.remove(&cookie, &header).is_err());
        assert_eq!(s.find(&cookie, &header, 101).as_deref(), Some("k1"));
        assert!(s.create("k1", 101, |_| true).is_err());
        assert_eq!(s.by_digest.len(), 1);
    }

    #[test]
    fn creating_a_session_drops_expired_ones_and_ones_of_unknown_keys() {
        let config = temp("prune");
        let mut s = Sessions::new(&config);
        let old = s.create("k1", 100, |_| true).unwrap();
        let gone = s.create("revoked", 100 + LIFETIME_SECS, |_| true).unwrap();
        s.create("k1", 101 + LIFETIME_SECS, |k| k == "k1").unwrap();
        assert_eq!(s.find(&old.0, &old.1, 50), None);
        assert_eq!(s.find(&gone.0, &gone.1, 102 + LIFETIME_SECS), None);
        assert_eq!(s.by_digest.len(), 1);
    }
}
