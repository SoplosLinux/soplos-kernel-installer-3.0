use serde::Serialize;
use std::fs;
use std::path::Path;
use std::process::Command;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HardwareInfo {
    pub cpu: String,
    /// The highest x86-64 level the processor supports: v1 to v4.
    pub march: String,
    pub gpu: String,
    pub ram_gb: f64,
}

/// The highest x86-64 level the processor supports, as the dynamic linker
/// itself reports it.
fn supported_march() -> String {
    for ld in ["/lib64/ld-linux-x86-64.so.2", "/lib/x86_64-linux-gnu/ld-linux-x86-64.so.2"] {
        if Path::new(ld).exists() {
            let out = Command::new(ld).arg("--help").output();
            if let Ok(out) = out {
                let text = String::from_utf8_lossy(&out.stdout).to_string() + &String::from_utf8_lossy(&out.stderr);
                return parse_march(&text);
            }
            break;
        }
    }
    "v1".to_string()
}

fn parse_march(help: &str) -> String {
    for level in ["v4", "v3", "v2"] {
        let marker = format!("x86-64-{level}");
        if help.lines().any(|l| l.trim_start().starts_with(&marker) && l.contains("(supported")) {
            return level.to_string();
        }
    }
    "v1".to_string()
}

/// The x86-64 level (1 to 4) the processor supports: the one the dynamic
/// linker reports, the same the window shows (supported_march), and the
/// flags only when the linker gives no answer. The flags alone gave v2 on an
/// Intel N150 under Proxmox that runs v3 kernels (2026-11-13), while the
/// window said v3.
pub fn psabi_level() -> u8 {
    for ld in ["/lib64/ld-linux-x86-64.so.2", "/lib/x86_64-linux-gnu/ld-linux-x86-64.so.2"] {
        if let Ok(out) = Command::new(ld).arg("--help").output() {
            let text = String::from_utf8_lossy(&out.stdout).to_string() + &String::from_utf8_lossy(&out.stderr);
            if text.contains("glibc-hwcaps") {
                return parse_march(&text)[1..].parse().unwrap_or(1);
            }
        }
    }
    let cpuinfo = fs::read_to_string("/proc/cpuinfo").unwrap_or_default();
    let flags = cpuinfo.lines().find(|l| l.starts_with("flags")).and_then(|l| l.split_once(':')).map(|(_, v)| v.to_string()).unwrap_or_default();
    psabi_from_flags(&flags)
}

fn psabi_from_flags(flags: &str) -> u8 {
    let set: std::collections::HashSet<&str> = flags.split_whitespace().collect();
    if set.is_empty() {
        return 1;
    }
    let has_all = |list: &[&str]| list.iter().all(|f| set.contains(f));
    if !has_all(&["cx16", "lahf_lm", "popcnt", "sse4_1", "sse4_2", "ssse3"]) {
        return 1;
    }
    if !has_all(&["avx", "avx2", "bmi1", "bmi2", "f16c", "fma", "abm", "movbe", "xsave"]) {
        return 2;
    }
    if !has_all(&["avx512f", "avx512bw", "avx512cd", "avx512dq", "avx512vl"]) {
        return 3;
    }
    4
}

pub(crate) fn cpu_model() -> String {
    fs::read_to_string("/proc/cpuinfo")
        .ok()
        .and_then(|t| t.lines().find(|l| l.starts_with("model name")).and_then(|l| l.split_once(':').map(|(_, v)| v.trim().to_string())))
        .unwrap_or_default()
}

fn gpu_models() -> String {
    let Ok(out) = Command::new("lspci").output() else { return String::new() };
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter(|l| {
            let lower = l.to_lowercase();
            ["vga", "display", "3d controller"].iter().any(|k| lower.contains(k))
        })
        .filter_map(|l| l.splitn(3, ':').nth(2).map(|m| m.trim().to_string()))
        .collect::<Vec<_>>()
        .join(", ")
}

fn ram_gb() -> f64 {
    fs::read_to_string("/proc/meminfo")
        .ok()
        .and_then(|t| t.lines().find(|l| l.starts_with("MemTotal:")).and_then(|l| l.split_whitespace().nth(1).and_then(|v| v.parse::<f64>().ok())))
        .map(|kb| kb / 1_048_576.0)
        .unwrap_or(0.0)
}

#[tauri::command(async)]
pub fn get_hardware_info() -> HardwareInfo {
    HardwareInfo { cpu: cpu_model(), march: supported_march(), gpu: gpu_models(), ram_gb: ram_gb() }
}

#[cfg(test)]
mod tests {
    use super::{parse_march, psabi_from_flags};

    #[test]
    fn highest_supported_level_wins() {
        let help = "Subdirectories of glibc-hwcaps directories, in priority order:\n  x86-64-v4\n  x86-64-v3 (supported, searched)\n  x86-64-v2 (supported, searched)\n";
        assert_eq!(parse_march(help), "v3");
        assert_eq!(parse_march("  x86-64-v4 (supported, searched)\n  x86-64-v3 (supported, searched)\n"), "v4");
        assert_eq!(parse_march("nothing"), "v1");
    }

    #[test]
    fn psabi_levels() {
        let v2 = "cx16 lahf_lm popcnt sse4_1 sse4_2 ssse3";
        let v3 = format!("{v2} avx avx2 bmi1 bmi2 f16c fma abm movbe xsave");
        let v4 = format!("{v3} avx512f avx512bw avx512cd avx512dq avx512vl");
        assert_eq!(psabi_from_flags(""), 1);
        assert_eq!(psabi_from_flags("sse2"), 1);
        assert_eq!(psabi_from_flags(v2), 2);
        assert_eq!(psabi_from_flags(&v3), 3);
        assert_eq!(psabi_from_flags(&v4), 4);
    }
}
