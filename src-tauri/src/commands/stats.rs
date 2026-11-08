// What a build costs the computer, shown while it runs: the load of the CPU, the
// memory, the disk of the folder of builds and the temperature of the CPU.
// Ported from `utils/system.py` and the stats of `widgets/build_progress.py` of
// the 2.x program. Nothing here changes anything: it only reads.

use crate::build::pipeline::build_dir;
use serde::Serialize;
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

// The drivers that give a real reading of the CPU, in order. k10temp (AMD Tctl)
// and coretemp (Intel) are the real ones. acpitz is the thermal zone of the
// board: often a stub with a fixed value, so it is the last resort. The order of
// /sys/class/hwmon is not stable, so the first match is not enough.
const HWMON_PRIORITY: [&str; 5] = ["k10temp", "coretemp", "nct6775", "it87", "acpitz"];

#[derive(Serialize, Debug, Default, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SystemStats {
    pub load1: f64,
    pub load5: f64,
    pub load15: f64,
    pub cpu_count: u32,
    pub mem_used_gb: f64,
    pub mem_total_gb: f64,
    pub mem_percent: f64,
    pub disk_free_gb: f64,
    pub disk_total_gb: f64,
    pub disk_percent: f64,
    /// Degrees Celsius; none when no sensor of the CPU was found.
    pub cpu_temp: Option<f64>,
}

fn load_average() -> (f64, f64, f64) {
    let text = fs::read_to_string("/proc/loadavg").unwrap_or_default();
    let mut parts = text.split_whitespace().map(|p| p.parse::<f64>().unwrap_or(0.0));
    (parts.next().unwrap_or(0.0), parts.next().unwrap_or(0.0), parts.next().unwrap_or(0.0))
}

/// (used GB, total GB, percent used) from the text of /proc/meminfo.
fn memory_from(text: &str) -> (f64, f64, f64) {
    let mut total: f64 = 0.0;
    let mut available: Option<f64> = None;
    let mut free: f64 = 0.0;
    for line in text.lines() {
        let mut it = line.split_whitespace();
        let (Some(key), Some(value)) = (it.next(), it.next()) else { continue };
        let kb = value.parse::<f64>().unwrap_or(0.0);
        match key.trim_end_matches(':') {
            "MemTotal" => total = kb,
            "MemAvailable" => available = Some(kb),
            "MemFree" => free = kb,
            _ => {}
        }
    }
    if total <= 0.0 {
        return (0.0, 0.0, 0.0);
    }
    let used = total - available.unwrap_or(free);
    (used / 1_048_576.0, total / 1_048_576.0, used / total * 100.0)
}

/// (free GB, total GB, percent used) of the disk that holds a folder.
fn disk_of(dir: &Path) -> (f64, f64, f64) {
    // The folder of builds may not exist yet: its parent is on the same disk.
    let mut target: PathBuf = dir.to_path_buf();
    while !target.exists() {
        if !target.pop() {
            break;
        }
    }
    let Ok(out) = Command::new("df").args(["-B1", "--output=size,avail"]).arg(&target).output() else { return (0.0, 0.0, 0.0) };
    let text = String::from_utf8_lossy(&out.stdout);
    let mut nums = text.lines().nth(1).unwrap_or("").split_whitespace().map(|n| n.parse::<f64>().unwrap_or(0.0));
    let (total, free) = (nums.next().unwrap_or(0.0), nums.next().unwrap_or(0.0));
    if total <= 0.0 {
        return (0.0, 0.0, 0.0);
    }
    (free / 1_073_741_824.0, total / 1_073_741_824.0, (total - free) / total * 100.0)
}

/// The best reading of the CPU in one hwmon folder: the input labelled Tctl (AMD)
/// or Package / Core 0 (Intel), else the one with the lowest number.
fn hwmon_reading(dir: &Path) -> Option<f64> {
    let mut inputs: BTreeMap<u32, f64> = BTreeMap::new();
    for entry in fs::read_dir(dir).ok()?.flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        let Some(index) = name.strip_prefix("temp").and_then(|r| r.strip_suffix("_input")).and_then(|i| i.parse::<u32>().ok()) else { continue };
        if let Some(milli) = fs::read_to_string(entry.path()).ok().and_then(|t| t.trim().parse::<f64>().ok()) {
            inputs.insert(index, milli / 1000.0);
        }
    }
    for (index, value) in &inputs {
        if let Ok(label) = fs::read_to_string(dir.join(format!("temp{index}_label"))) {
            let label = label.trim().to_lowercase();
            if ["tctl", "package", "core 0"].iter().any(|k| label.contains(k)) {
                return Some(*value);
            }
        }
    }
    inputs.values().next().copied()
}

fn cpu_temperature(base: &Path) -> Option<f64> {
    let mut by_driver: BTreeMap<String, Vec<PathBuf>> = BTreeMap::new();
    for entry in fs::read_dir(base).ok()?.flatten() {
        let path = entry.path();
        if let Ok(name) = fs::read_to_string(path.join("name")) {
            by_driver.entry(name.trim().to_string()).or_default().push(path);
        }
    }
    for driver in HWMON_PRIORITY {
        if let Some(paths) = by_driver.get_mut(driver) {
            paths.sort();
            for path in paths.iter() {
                if let Some(temp) = hwmon_reading(path) {
                    return Some(temp);
                }
            }
        }
    }
    None
}

/// The numbers of the panel of a build, read now.
#[tauri::command(async)]
pub fn get_system_stats() -> SystemStats {
    let (load1, load5, load15) = load_average();
    let (mem_used_gb, mem_total_gb, mem_percent) = memory_from(&fs::read_to_string("/proc/meminfo").unwrap_or_default());
    let (disk_free_gb, disk_total_gb, disk_percent) = disk_of(&build_dir());
    SystemStats {
        load1,
        load5,
        load15,
        cpu_count: std::thread::available_parallelism().map(|n| n.get() as u32).unwrap_or(1),
        mem_used_gb,
        mem_total_gb,
        mem_percent,
        disk_free_gb,
        disk_total_gb,
        disk_percent,
        cpu_temp: cpu_temperature(Path::new("/sys/class/hwmon")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn memory_uses_what_is_available() {
        let text = "MemTotal:       16000000 kB\nMemFree:         1000000 kB\nMemAvailable:    8000000 kB\n";
        let (used, total, percent) = memory_from(text);
        assert!((total - 15.258).abs() < 0.01);
        assert!((used - 7.629).abs() < 0.01);
        assert!((percent - 50.0).abs() < 0.01);
        // Without MemAvailable, MemFree is what is left.
        let (_, _, percent) = memory_from("MemTotal: 1000 kB\nMemFree: 250 kB\n");
        assert!((percent - 75.0).abs() < 0.01);
        assert_eq!(memory_from(""), (0.0, 0.0, 0.0));
    }

    #[test]
    fn the_real_driver_wins_over_the_stub() {
        let base = std::env::temp_dir().join(format!("ki-hwmon-{}", std::process::id()));
        let _ = fs::remove_dir_all(&base);
        for (dir, name, temp, label) in [("hwmon0", "acpitz", "20000", None), ("hwmon1", "k10temp", "61500", Some("Tctl"))] {
            let d = base.join(dir);
            fs::create_dir_all(&d).unwrap();
            fs::write(d.join("name"), format!("{name}\n")).unwrap();
            fs::write(d.join("temp1_input"), format!("{temp}\n")).unwrap();
            if let Some(l) = label {
                fs::write(d.join("temp1_label"), format!("{l}\n")).unwrap();
            }
        }
        assert_eq!(cpu_temperature(&base), Some(61.5));
        let _ = fs::remove_dir_all(&base);
        assert_eq!(cpu_temperature(&base), None);
    }
}
