// What the assistant of Build asks the program: the kernels that can be built,
// what this computer is like, and the build itself.

use crate::build::batch::{self, BatchJob, BatchSpec};
use crate::build::deps::{self, Deps};
use crate::build::nvidia;
use crate::build::pipeline::{self, Snapshot};
use crate::build::recipe::{self, Recipe};
use crate::build::versions::{self, KernelVersion};
use crate::commands::hardware::{cpu_model, psabi_level};
use serde::Serialize;
use std::fs;
use std::path::Path;
use std::process::Command;
use tauri::AppHandle;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Goal {
    pub id: String,
    pub patches: Vec<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NvidiaInfo {
    pub present: bool,
    /// The version of the driver whose sources DKMS builds, e.g. "590.48.01".
    pub driver: Option<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BuildEnvironment {
    pub cpu: Option<String>,
    pub cpu_threads: u32,
    /// The highest x86-64 level of the processor: "v1" to "v4".
    pub march: String,
    pub free_bytes: u64,
    pub build_dir: String,
    pub deps: Deps,
    pub nvidia: NvidiaInfo,
    pub keys_exist: bool,
    /// The processor is an AMD X3D, which the V-Cache patch is for.
    pub x3d: bool,
    pub goals: Vec<Goal>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BuildSources {
    pub exists: bool,
    pub size: u64,
}

fn free_bytes(dir: &Path) -> u64 {
    // The folder may not exist yet: ask about the first one that does.
    let mut probe = dir.to_path_buf();
    while !probe.exists() {
        if !probe.pop() {
            break;
        }
    }
    let out = Command::new("df").args(["-B1", "--output=avail"]).arg(&probe).output();
    out.ok()
        .and_then(|o| String::from_utf8_lossy(&o.stdout).lines().nth(1).and_then(|l| l.trim().parse::<u64>().ok()))
        .unwrap_or(0)
}

// The sources DKMS builds are in /usr/src/nvidia-<version>.
fn driver_version() -> Option<String> {
    let mut found: Vec<String> = fs::read_dir("/usr/src")
        .ok()?
        .filter_map(|e| e.ok())
        .map(|e| e.file_name().to_string_lossy().to_string())
        .filter(|n| n.starts_with("nvidia"))
        .filter_map(|n| n.rsplit('-').next().map(str::to_string))
        .filter(|v| v.starts_with(|c: char| c.is_ascii_digit()) && v.contains('.'))
        .collect();
    found.sort();
    found.pop()
}

#[tauri::command(async)]
pub fn list_kernel_versions() -> Result<Vec<KernelVersion>, String> {
    versions::fetch()
}

#[tauri::command(async)]
pub fn get_build_environment() -> BuildEnvironment {
    let cpu = Some(cpu_model()).filter(|c| !c.is_empty());
    let dir = pipeline::build_dir();
    let present = nvidia::has_nvidia_gpu();
    BuildEnvironment {
        x3d: cpu.as_deref().map(|c| c.to_lowercase().contains("x3d")).unwrap_or(false),
        cpu,
        cpu_threads: std::thread::available_parallelism().map(|n| n.get() as u32).unwrap_or(1),
        march: format!("v{}", psabi_level().clamp(1, 4)),
        free_bytes: free_bytes(&dir),
        build_dir: dir.to_string_lossy().to_string(),
        deps: deps::check(),
        nvidia: NvidiaInfo { present, driver: if present { driver_version() } else { None } },
        keys_exist: pipeline::keys_file().is_file(),
        goals: recipe::GOALS
            .iter()
            .map(|g| Goal { id: g.to_string(), patches: recipe::suggested_patches(g).iter().map(|p| p.to_string()).collect() })
            .collect(),
    }
}

#[tauri::command]
pub fn start_build(app: AppHandle, recipe: Recipe) -> Result<(), String> {
    pipeline::start(app, recipe)
}

#[tauri::command]
pub fn cancel_build() {
    pipeline::cancel();
}

#[tauri::command]
pub fn get_build_job() -> Snapshot {
    pipeline::snapshot()
}

#[tauri::command(async)]
pub fn get_build_sources(version: String) -> BuildSources {
    let size = pipeline::files_size(&version);
    let exists = pipeline::files_of(&version).map(|(source, _)| source.join("Makefile").is_file()).unwrap_or(false);
    BuildSources { exists, size }
}

/// Removes the sources and the download of a version. Returns what it freed.
#[tauri::command(async)]
pub fn clean_build_files(version: String) -> Result<u64, String> {
    pipeline::clean_files(&version)
}

/// Deletes the folder of the builds (`~/kernel_build`) after a build was
/// cancelled. Returns what it freed.
#[tauri::command(async)]
pub fn clean_build_dir() -> Result<u64, String> {
    pipeline::remove_build_dir()
}

/// The kernels of a release, for a channel ("", "rc", "lts" or "eol").
#[tauri::command]
pub fn get_batch_queue(channel: String) -> Result<Vec<BatchJob>, String> {
    if !["", "rc", "lts", "eol"].contains(&channel.as_str()) {
        return Err(format!("Unknown channel: {channel}"));
    }
    Ok(batch::release_queue(&channel))
}

#[tauri::command]
pub fn start_batch(app: AppHandle, spec: BatchSpec) -> Result<(), String> {
    batch::start(app, spec)
}

/// Which of the kernels are already in the destination.
#[tauri::command(async)]
pub fn batch_built(dest: String, version: String, channel: String, revision: u32) -> Vec<String> {
    let root = std::path::PathBuf::from(&dest);
    if !root.is_absolute() || !crate::build::recipe::valid_version(&version) {
        return Vec::new();
    }
    batch::release_queue(&channel).into_iter().filter(|j| batch::already_built(&root, &version, j, revision)).map(|j| j.name).collect()
}
