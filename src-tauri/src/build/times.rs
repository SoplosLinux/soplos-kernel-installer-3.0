// The history of how long each kernel took to build, to know what a queue will
// cost and how a computer compares with another. One entry for every kernel that
// was built whole, in a batch or on its own. It only keeps the last ones.

use crate::history::{data_dir, now_iso};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

const KEEP: usize = 500;

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct BuildTime {
    pub name: String,
    pub version: String,
    pub seconds: u64,
    /// When it finished, as `history` writes the dates.
    pub date: String,
    pub cores: u32,
}

fn file() -> PathBuf {
    data_dir().join("build-times.json")
}

/// Oldest first, as they are kept.
pub fn load() -> Vec<BuildTime> {
    fs::read_to_string(file()).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default()
}

/// Adds one to the history. A history that cannot be written is not a failure of
/// the build.
pub fn record(name: &str, version: &str, seconds: u64, cores: u32) {
    let mut all = load();
    all.push(BuildTime { name: name.to_string(), version: version.to_string(), seconds, date: now_iso(), cores });
    if all.len() > KEEP {
        let drop = all.len() - KEEP;
        all.drain(..drop);
    }
    if fs::create_dir_all(data_dir()).is_ok() {
        if let Ok(text) = serde_json::to_string_pretty(&all) {
            let _ = fs::write(file(), text);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_entry_reads_back_as_written() {
        let one = BuildTime { name: "soplos-v1".into(), version: "7.3.3".into(), seconds: 754, date: "2026-11-08T10:00:00".into(), cores: 8 };
        let text = serde_json::to_string(&one).unwrap();
        assert!(text.contains("\"seconds\":754"));
        assert_eq!(serde_json::from_str::<BuildTime>(&text).unwrap(), one);
    }
}
