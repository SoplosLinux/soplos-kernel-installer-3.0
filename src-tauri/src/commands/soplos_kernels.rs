// The ready-made kernels of Soplos, which come from the apt repository of
// Soplos Kernels: what is listed, and what is installed, removed or updated.

use crate::privileged::{self, valid_kernel_package, Op, REPO_FILE};
use serde::Serialize;
use std::collections::HashMap;
use std::fs;
use std::process::Command;
use tauri::{AppHandle, Emitter};

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct KernelPackage {
    pub package: String,
    /// "Stock", "BORE", "Zen"... read from the name.
    pub profile: String,
    /// "V1" to "V4", or empty when the package has no level.
    pub march: String,
    pub description: String,
    pub version: String,
    pub installed: bool,
    pub update_available: bool,
    /// The kernel that is running now is this one.
    pub is_running: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RepoState {
    pub configured: bool,
    pub packages: Vec<KernelPackage>,
    /// The release of the kernel that is running now.
    pub running: String,
}

/// Splits `linux-soplos-bore-v3` into its profile and its level.
pub fn flavor_and_march(package: &str) -> (String, String) {
    let suffix = package.strip_prefix("linux-soplos").unwrap_or("");
    let parts: Vec<&str> = suffix.trim_start_matches('-').split('-').filter(|p| !p.is_empty()).collect();
    let march = match parts.last() {
        Some(p) if matches!(*p, "v1" | "v2" | "v3" | "v4") => p.to_uppercase(),
        _ => String::new(),
    };
    let flavor: Vec<&str> = if march.is_empty() { parts.clone() } else { parts[..parts.len() - 1].to_vec() };
    let label = if flavor.is_empty() {
        "Stock".to_string()
    } else {
        let joined = flavor.join("-");
        if joined.len() <= 5 {
            joined.to_uppercase()
        } else {
            flavor.iter().map(|w| capitalize(w)).collect::<Vec<_>>().join(" ")
        }
    };
    (label, march)
}

fn capitalize(word: &str) -> String {
    let mut chars = word.chars();
    chars.next().map(|f| f.to_uppercase().collect::<String>() + chars.as_str()).unwrap_or_default()
}

fn apt(args: &[&str]) -> String {
    Command::new("apt-cache")
        .args(args)
        .env("LC_ALL", "C")
        .env("LANG", "C")
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).to_string())
        .unwrap_or_default()
}

struct Stanza {
    package: String,
    version: String,
    description: String,
    suffix: String,
}

// The first stanza of every package is its newest version.
fn parse_show(text: &str) -> Vec<Stanza> {
    let mut seen: HashMap<String, ()> = HashMap::new();
    let mut out = Vec::new();
    for block in text.split("\n\n") {
        let mut s = Stanza { package: String::new(), version: String::new(), description: String::new(), suffix: String::new() };
        for line in block.lines() {
            if let Some(v) = line.strip_prefix("Package:") {
                s.package = v.trim().to_string();
            } else if let Some(v) = line.strip_prefix("Version:") {
                s.version = v.trim().to_string();
            } else if let Some(v) = line.strip_prefix("Description:") {
                s.description = v.trim().to_string();
            } else if let Some(deps) = line.strip_prefix("Depends:") {
                for dep in deps.split(',') {
                    let name = dep.split_whitespace().next().unwrap_or("");
                    if name.starts_with("linux-image-") {
                        if let Some(idx) = name.find("-soplos") {
                            s.suffix = name[idx..].to_string();
                        }
                        break;
                    }
                }
            }
        }
        if s.package.is_empty() || seen.contains_key(&s.package) {
            continue;
        }
        seen.insert(s.package.clone(), ());
        out.push(s);
    }
    out
}

// package -> (installed version, candidate version)
fn parse_policy(text: &str) -> HashMap<String, (String, String)> {
    let mut out: HashMap<String, (String, String)> = HashMap::new();
    let mut current = String::new();
    for line in text.lines() {
        if !line.starts_with(' ') && line.ends_with(':') {
            current = line.trim_end_matches(':').to_string();
            out.entry(current.clone()).or_default();
        } else if let Some(v) = line.trim().strip_prefix("Installed:") {
            let v = v.trim();
            if v != "(none)" {
                out.entry(current.clone()).or_default().0 = v.to_string();
            }
        } else if let Some(v) = line.trim().strip_prefix("Candidate:") {
            let v = v.trim();
            if v != "(none)" {
                out.entry(current.clone()).or_default().1 = v.to_string();
            }
        }
    }
    out
}

fn boot_images() -> Vec<String> {
    fs::read_dir("/boot")
        .map(|d| d.filter_map(|e| e.ok()).map(|e| e.file_name().to_string_lossy().to_string()).filter(|n| n.starts_with("vmlinuz-")).collect())
        .unwrap_or_default()
}

#[tauri::command(async)]
pub fn get_repo_state() -> RepoState {
    let configured = std::path::Path::new(REPO_FILE).exists();
    let running = fs::read_to_string("/proc/sys/kernel/osrelease").unwrap_or_default().trim().to_string();

    let names: Vec<String> = apt(&["pkgnames", "linux-soplos"])
        .lines()
        .map(|l| l.trim().to_string())
        .filter(|n| valid_kernel_package(n))
        .collect();
    if names.is_empty() {
        return RepoState { configured, packages: Vec::new(), running };
    }

    let mut show_args = vec!["show"];
    show_args.extend(names.iter().map(String::as_str));
    let stanzas = parse_show(&apt(&show_args));
    let mut policy_args = vec!["policy"];
    policy_args.extend(names.iter().map(String::as_str));
    let policy = parse_policy(&apt(&policy_args));
    let images = boot_images();

    let mut packages: Vec<KernelPackage> = stanzas
        .into_iter()
        // A package without a description is not a kernel of the list.
        .filter(|s| !s.description.is_empty())
        .map(|s| {
            let (installed_ver, candidate_ver) = policy.get(&s.package).cloned().unwrap_or_default();
            let installed = !s.suffix.is_empty() && images.iter().any(|n| n.ends_with(&s.suffix));
            let mut update_available = !installed_ver.is_empty() && !candidate_ver.is_empty() && installed_ver != candidate_ver;
            // A kernel that was put there without apt knowing it: compare the
            // version of its image with the one of the repository.
            if !update_available && installed && installed_ver.is_empty() {
                if let Some(image) = images.iter().find(|n| n.ends_with(&s.suffix)) {
                    let boot_ver = image["vmlinuz-".len()..image.len() - s.suffix.len()].to_string();
                    update_available = !boot_ver.is_empty() && !s.version.is_empty() && boot_ver != s.version;
                }
            }
            let is_running = !s.suffix.is_empty() && running.ends_with(&s.suffix);
            let (profile, march) = flavor_and_march(&s.package);
            KernelPackage { package: s.package, profile, march, description: s.description, version: s.version, installed, update_available, is_running }
        })
        .collect();
    packages.sort_by(|a, b| (a.package != "linux-soplos", &a.package).cmp(&(b.package != "linux-soplos", &b.package)));
    RepoState { configured, packages, running }
}

fn run_ops(app: &AppHandle, ops: Vec<Op>) -> Result<(), String> {
    privileged::run_as_root(&ops, |line| {
        let _ = app.emit("ops-line", line.to_string());
    })
}

#[tauri::command(async)]
pub fn repo_add(app: AppHandle) -> Result<(), String> {
    run_ops(&app, vec![Op::AddKernelRepo])
}

#[tauri::command(async)]
pub fn repo_remove(app: AppHandle) -> Result<(), String> {
    run_ops(&app, vec![Op::RemoveKernelRepo])
}

#[tauri::command(async)]
pub fn repo_refresh(app: AppHandle) -> Result<(), String> {
    run_ops(&app, vec![Op::AptUpdate])
}

/// `action` is "install", "remove" or "update".
#[tauri::command(async)]
pub fn kernel_package_action(app: AppHandle, package: String, action: String) -> Result<(), String> {
    if !valid_kernel_package(&package) {
        return Err(format!("Not a kernel package of Soplos: {package}"));
    }
    let op = match action.as_str() {
        "install" => Op::InstallKernel { package },
        "remove" => Op::RemoveKernel { package },
        "update" => Op::UpgradeKernel { package },
        _ => return Err(format!("Unknown action: {action}")),
    };
    run_ops(&app, vec![op])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_are_split_into_profile_and_level() {
        assert_eq!(flavor_and_march("linux-soplos"), ("Stock".into(), "".into()));
        assert_eq!(flavor_and_march("linux-soplos-v3"), ("Stock".into(), "V3".into()));
        assert_eq!(flavor_and_march("linux-soplos-bore-v4"), ("BORE".into(), "V4".into()));
        assert_eq!(flavor_and_march("linux-soplos-zen"), ("ZEN".into(), "".into()));
        assert_eq!(flavor_and_march("linux-soplos-bore-ntsync-v3"), ("Bore Ntsync".into(), "V3".into()));
    }

    #[test]
    fn policy_is_read_per_package() {
        let text = "linux-soplos:\n  Installed: 1.0\n  Candidate: 1.1\n  Version table:\nlinux-soplos-zen:\n  Installed: (none)\n  Candidate: 2.0\n";
        let p = parse_policy(text);
        assert_eq!(p["linux-soplos"], ("1.0".to_string(), "1.1".to_string()));
        assert_eq!(p["linux-soplos-zen"], (String::new(), "2.0".to_string()));
    }

    #[test]
    fn only_the_newest_stanza_of_a_package_counts() {
        let text = "Package: linux-soplos\nVersion: 2\nDescription: A\nDepends: linux-image-7.3.1-soplos, x\n\nPackage: linux-soplos\nVersion: 1\nDescription: A\n";
        let s = parse_show(text);
        assert_eq!(s.len(), 1);
        assert_eq!(s[0].version, "2");
        assert_eq!(s[0].suffix, "-soplos");
    }
}
