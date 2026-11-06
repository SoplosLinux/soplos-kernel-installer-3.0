// The kernels that are installed on this computer, and removing one.

use crate::history;
use crate::privileged::{self, valid_release, Op};
use serde::Serialize;
use std::fs;
use tauri::{AppHandle, Emitter};

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InstalledKernel {
    pub version: String,
    /// "soplos", "gaming", "audio", "minimal", "automatic", "stock", "xanmod",
    /// "liquorix", "zen", "rt", "lowlatency", "cloud" or "system".
    pub profile: String,
    pub patches: String,
    /// "YYYY-MM-DD".
    pub date: String,
    pub is_current: bool,
    pub secure_boot: bool,
}

// The profile of a kernel, told from its release.
pub fn profile_of(release: &str) -> &'static str {
    let v = release.to_lowercase();
    // The suffixes of the profiles of the program, in the order of the 2.x one.
    for (suffix, id) in [("gaming", "gaming"), ("lowlatency", "audio"), ("minimal", "minimal"), ("optimized", "automatic")] {
        if v.ends_with(&format!("-{suffix}")) {
            return id;
        }
    }
    if v.contains("soplos") {
        return "soplos";
    }
    for (keyword, id) in [("xanmod", "xanmod"), ("liquorix", "liquorix"), ("zen", "zen"), ("rt", "rt"), ("lowlatency", "lowlatency"), ("cloud", "cloud")] {
        if v.contains(keyword) {
            return id;
        }
    }
    "system"
}

fn mtime_date(path: &std::path::Path) -> String {
    fs::metadata(path)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| history::date_of(d.as_secs() as i64))
        .unwrap_or_default()
}

#[tauri::command(async)]
pub fn list_installed_kernels() -> Vec<InstalledKernel> {
    let running = fs::read_to_string("/proc/sys/kernel/osrelease").unwrap_or_default().trim().to_string();
    let remembered = history::load();
    let mut out: Vec<InstalledKernel> = Vec::new();
    let Ok(dir) = fs::read_dir("/boot") else { return out };
    for entry in dir.filter_map(|e| e.ok()) {
        let name = entry.file_name().to_string_lossy().to_string();
        let Some(version) = name.strip_prefix("vmlinuz-") else { continue };
        if version.is_empty() {
            continue;
        }
        let known = remembered.iter().find(|e| e.version.eq_ignore_ascii_case(version));
        let date = known
            .map(|k| k.installed_date.chars().take(10).collect::<String>())
            .filter(|d| !d.is_empty())
            .unwrap_or_else(|| mtime_date(&entry.path()));
        out.push(InstalledKernel {
            version: version.to_string(),
            profile: profile_of(version).to_string(),
            patches: known.map(|k| k.patches.clone()).filter(|p| p != "none").unwrap_or_default(),
            date,
            is_current: version == running,
            secure_boot: known.map(|k| k.secure_boot).unwrap_or(false),
        });
    }
    // The newest first.
    out.sort_by(|a, b| b.date.cmp(&a.date).then_with(|| privileged::version_cmp(&b.version, &a.version)));
    out
}

#[tauri::command(async)]
pub fn remove_installed_kernel(app: AppHandle, release: String) -> Result<(), String> {
    if !valid_release(&release) {
        return Err(format!("Not a valid kernel release: {release}"));
    }
    privileged::run_as_root(&[Op::RemoveInstalledKernel { release: release.clone() }], |line| {
        let _ = app.emit("ops-line", line.to_string());
    })?;
    // Only what the user owns is changed here: the history of the program.
    let _ = history::remove(&release);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::profile_of;

    #[test]
    fn profiles_are_told_from_the_release() {
        assert_eq!(profile_of("7.3.1-gaming"), "gaming");
        assert_eq!(profile_of("7.3.1-lowlatency"), "audio");
        assert_eq!(profile_of("7.3.1-soplos-bore-ntsync-v4"), "soplos");
        assert_eq!(profile_of("6.12.9-xanmod1"), "xanmod");
        assert_eq!(profile_of("6.12.9+deb13-amd64"), "system");
        assert_eq!(profile_of("7.3.1-optimized"), "automatic");
    }
}
