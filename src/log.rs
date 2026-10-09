use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

/// `jeff.log` sits next to the config file, so one `--config` directory holds both.
pub fn path(config_path: &str) -> PathBuf {
    Path::new(config_path).with_file_name("jeff.log")
}

/// `YYYY-MM-DDTHH:MM:SSZ`; the date comes from Howard Hinnant's `civil_from_days`.
pub fn utc(secs: u64) -> String {
    let days = (secs / 86_400) as i64;
    let rem = secs % 86_400;
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}Z",
        rem / 3_600,
        rem % 3_600 / 60,
        rem % 60
    )
}

/// Appends one entry. A write failure is ignored: the console line still reports the problem.
pub fn write(config_path: &str, event: &str, summary: &str, problems: &[String]) {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    let mut entry = format!("{} {event} {summary}\n", utc(now));
    for p in problems {
        entry.push_str(&format!("  {p}\n"));
    }
    let _ = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path(config_path))
        .and_then(|mut f| f.write_all(entry.as_bytes()));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn utc_formats_seconds() {
        assert_eq!(utc(0), "1970-01-01T00:00:00Z");
        assert_eq!(utc(1_791_542_537), "2026-10-09T10:42:17Z");
    }

    #[test]
    fn path_sits_next_to_the_config() {
        assert_eq!(path("dir/jeff.json"), PathBuf::from("dir/jeff.log"));
        assert_eq!(path("jeff.json"), PathBuf::from("jeff.log"));
    }

    #[test]
    fn write_appends_entries() {
        let dir = std::env::temp_dir().join(format!("jeff-log-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let config = dir.join("jeff.json");
        let config = config.to_str().unwrap();
        write(
            config,
            "config_invalid",
            "jeff.json: 2 problems",
            &["a".into(), "b".into()],
        );
        write(config, "config_fixed", "jeff.json fixed", &[]);
        let text = fs::read_to_string(dir.join("jeff.log")).unwrap();
        let _ = fs::remove_dir_all(&dir);
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines.len(), 4, "{text}");
        assert!(
            lines[0].ends_with("Z config_invalid jeff.json: 2 problems"),
            "{text}"
        );
        assert_eq!(lines[1], "  a");
        assert_eq!(lines[2], "  b");
        assert!(
            lines[3].ends_with("Z config_fixed jeff.json fixed"),
            "{text}"
        );
    }
}
