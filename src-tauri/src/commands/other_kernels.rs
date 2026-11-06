// The kernels of Debian, XanMod and Liquorix: what is installed, and what is
// installed or removed.

use crate::commands::hardware::psabi_level;
use crate::kernel_sources::{self as ks, xanmod_package, DEBIAN_KERNELS, XANMOD_VARIANTS};
use crate::privileged::{self, Op};
use serde::Serialize;
use std::fs;
use std::process::Command;
use tauri::{AppHandle, Emitter};

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DebianKernel {
    pub package: String,
    pub installed: bool,
    pub in_use: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct XanmodVariant {
    pub key: String,
    /// None when the processor is too old for this variant.
    pub package: Option<String>,
    pub installed: bool,
    pub in_use: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OtherKernels {
    pub debian: Vec<DebianKernel>,
    pub xanmod: Vec<XanmodVariant>,
    pub xanmod_broken_repo: bool,
    pub psabi_level: u8,
    pub liquorix_installed: bool,
    pub liquorix_in_use: bool,
}

fn installed(package: &str) -> bool {
    Command::new("dpkg-query")
        .args(["-W", "-f=${db:Status-Abbrev}", package])
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).starts_with("ii"))
        .unwrap_or(false)
}

#[tauri::command(async)]
pub fn get_other_kernels() -> OtherKernels {
    let running = fs::read_to_string("/proc/sys/kernel/osrelease").unwrap_or_default().trim().to_string();
    let level = psabi_level();
    let own_flavor = running.contains("soplos") || running.contains("liquorix") || running.contains("xanmod");

    let debian = DEBIAN_KERNELS
        .iter()
        .map(|package| {
            let is_installed = installed(package);
            let rt = package.contains("-rt-");
            let in_use = is_installed && !own_flavor && running.ends_with("-amd64") && running.contains("-rt-") == rt;
            DebianKernel { package: package.to_string(), installed: is_installed, in_use }
        })
        .collect();

    let xanmod = XANMOD_VARIANTS
        .iter()
        .map(|key| {
            let package = xanmod_package(key, level);
            let is_installed = package.as_deref().map(installed).unwrap_or(false);
            XanmodVariant { key: key.to_string(), package, installed: is_installed, in_use: is_installed && running.contains("xanmod") }
        })
        .collect();

    let liquorix_installed = installed("linux-image-liquorix-amd64");
    OtherKernels {
        debian,
        xanmod,
        xanmod_broken_repo: ks::xanmod_repo_is_broken(),
        psabi_level: level,
        liquorix_installed,
        liquorix_in_use: liquorix_installed && running.contains("liquorix"),
    }
}

fn run_ops(app: &AppHandle, op: Op) -> Result<(), String> {
    privileged::run_as_root(&[op], |line| {
        let _ = app.emit("ops-line", line.to_string());
    })
}

/// `action` is "install" or "remove".
#[tauri::command(async)]
pub fn debian_kernel_action(app: AppHandle, package: String, action: String) -> Result<(), String> {
    if !DEBIAN_KERNELS.contains(&package.as_str()) {
        return Err(format!("Not a kernel of Debian: {package}"));
    }
    match action.as_str() {
        "install" => run_ops(&app, Op::InstallDebianKernel { package }),
        "remove" => run_ops(&app, Op::RemoveDebianKernel { package }),
        _ => Err(format!("Unknown action: {action}")),
    }
}

/// `action` is "install", "remove" or "repair" (the variant is then ignored).
#[tauri::command(async)]
pub fn xanmod_action(app: AppHandle, variant: String, action: String) -> Result<(), String> {
    match action.as_str() {
        "repair" => run_ops(&app, Op::RepairXanmodRepo),
        "install" | "remove" => {
            if !XANMOD_VARIANTS.contains(&variant.as_str()) {
                return Err(format!("Not a XanMod variant: {variant}"));
            }
            if action == "install" {
                run_ops(&app, Op::InstallXanmod { variant })
            } else {
                run_ops(&app, Op::RemoveXanmod { variant })
            }
        }
        _ => Err(format!("Unknown action: {action}")),
    }
}

/// `action` is "install" or "remove".
#[tauri::command(async)]
pub fn liquorix_action(app: AppHandle, action: String) -> Result<(), String> {
    match action.as_str() {
        "install" => run_ops(&app, Op::InstallLiquorix),
        "remove" => run_ops(&app, Op::RemoveLiquorix),
        _ => Err(format!("Unknown action: {action}")),
    }
}
