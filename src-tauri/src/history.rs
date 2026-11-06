// What the program remembers about the kernels it built: kept in the same
// file and with the same fields as the 2.x program, so the history of a
// computer carries over.

use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

#[derive(Serialize, Deserialize, Clone, Default)]
pub struct Entry {
    pub version: String,
    #[serde(default)]
    pub profile: String,
    #[serde(default)]
    pub patches: String,
    #[serde(default)]
    pub installed_date: String,
    #[serde(default)]
    pub secure_boot: bool,
}

pub fn data_dir() -> PathBuf {
    let base = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/share")))
        .unwrap_or_else(|| PathBuf::from("/tmp"));
    base.join("soplos-kernel-installer")
}

fn file() -> PathBuf {
    data_dir().join("history.json")
}

pub fn load() -> Vec<Entry> {
    fs::read_to_string(file()).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default()
}

fn store(entries: &[Entry]) -> Result<(), String> {
    fs::create_dir_all(data_dir()).map_err(|e| e.to_string())?;
    let text = serde_json::to_string_pretty(entries).map_err(|e| e.to_string())?;
    fs::write(file(), text).map_err(|e| e.to_string())
}

/// Remembers a kernel that was built and installed, the newest first. The list
/// keeps the last twenty, as the 2.x program did.
pub fn save(entry: Entry) -> Result<(), String> {
    let mut entries: Vec<Entry> = load().into_iter().filter(|e| !e.version.eq_ignore_ascii_case(&entry.version)).collect();
    entries.insert(0, entry);
    entries.truncate(20);
    store(&entries)
}

/// Forgets a kernel that was removed.
pub fn remove(version: &str) -> Result<(), String> {
    let entries = load();
    let kept: Vec<Entry> = entries.iter().filter(|e| e.version != version).cloned().collect();
    if kept.len() == entries.len() {
        return Ok(());
    }
    store(&kept)
}

/// The moment now, as an ISO date and time (UTC), the way the history keeps it.
pub fn now_iso() -> String {
    let secs = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0);
    format!("{}T{:02}:{:02}:{:02}", date_of(secs), (secs.rem_euclid(86_400)) / 3600, (secs.rem_euclid(3600)) / 60, secs.rem_euclid(60))
}

/// The date of a Unix time as "YYYY-MM-DD" (UTC).
pub fn date_of(secs: i64) -> String {
    // Days since 1970-01-01 to a civil date (Howard Hinnant's algorithm).
    let z = secs.div_euclid(86_400) + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    format!("{:04}-{:02}-{:02}", if m <= 2 { y + 1 } else { y }, m, d)
}

#[cfg(test)]
mod tests {
    use super::date_of;

    #[test]
    fn dates() {
        assert_eq!(date_of(0), "1970-01-01");
        assert_eq!(date_of(1_794_000_000), "2026-11-06");
        assert_eq!(date_of(951_782_400), "2000-02-29");
    }
}
