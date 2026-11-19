// The build, from the download to the packages, and what the window can ask
// about it. It runs as the user, in `~/kernel_build` as the 2.x program did;
// only the install at the end asks for the administrator. Ported from
// `KernelManager.full_install` and the downloader of the 2.x program.

use super::config::{self, Level};
use super::patches;
use super::recipe::{self, Recipe};
use super::runner::{run_capture, run_stream};
use super::deps;
use crate::history;
use crate::privileged::{self, Op};
use serde::Serialize;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};
use tauri::{AppHandle, Emitter};

static CANCEL: AtomicBool = AtomicBool::new(false);
static STATE: OnceLock<Mutex<Snapshot>> = OnceLock::new();
static LOG: Mutex<Option<std::fs::File>> = Mutex::new(None);

const MAX_LINES: usize = 600;
const DEBIAN_PACKAGES: &str = "https://deb.debian.org/debian/dists/sid/main/binary-amd64/Packages.xz";
const DEBIAN_POOL: &str = "https://deb.debian.org/debian/";

#[derive(Serialize, Clone, Debug, Default)]
#[serde(rename_all = "camelCase")]
pub struct Snapshot {
    pub running: bool,
    pub stage: String,
    pub percent: f32,
    pub lines: Vec<String>,
    /// "done", "failed" or "cancelled" once it ended.
    pub result: Option<String>,
    pub error: Option<String>,
    pub release: Option<String>,
    pub recipe: Option<Recipe>,
    /// What DKMS says about the NVIDIA module of the new kernel.
    pub nvidia: Option<String>,
    /// Set while a batch of kernels is being built, or after one.
    pub batch: Option<BatchProgress>,
}

#[derive(Serialize, Clone, Debug, Default)]
#[serde(rename_all = "camelCase")]
pub struct BatchProgress {
    /// The kernel being built, counted from 1.
    pub index: u32,
    pub total: u32,
    pub name: String,
    pub completed: Vec<String>,
    /// How long each of the finished kernels took, in seconds, in the same order.
    pub timings: Vec<u64>,
    /// When the kernel being built started, in milliseconds since 1970.
    pub started_at: u64,
    pub dest: String,
    /// The folder with the sources and the log of the kernel that failed.
    pub failed: Option<String>,
}

pub(crate) fn state() -> &'static Mutex<Snapshot> {
    STATE.get_or_init(|| Mutex::new(Snapshot::default()))
}

pub fn snapshot() -> Snapshot {
    state().lock().map(|s| s.clone()).unwrap_or_default()
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct Event {
    stage: String,
    percent: f32,
    /// "info", "warn" or "error".
    level: String,
    line: String,
}

pub(crate) struct Ctx {
    pub(crate) app: AppHandle,
}

/// What a build is asked for besides its recipe.
pub(crate) struct Options {
    /// The Debian revision of the packages (KDEB_PKGVERSION).
    pub kdeb: u32,
    /// Keep a download that is already there and matches its hash.
    pub keep_tarball: bool,
    /// Look for the tools of the build (a batch does it once).
    pub check_deps: bool,
}

impl Default for Options {
    fn default() -> Self {
        Options { kdeb: 1, keep_tarball: false, check_deps: true }
    }
}

/// Starts the log of a build, `~/kernel_build/build.log`, empty.
pub(crate) fn reset_log() {
    let dir = build_dir();
    let _ = fs::create_dir_all(&dir);
    if let Ok(mut log) = LOG.lock() {
        *log = fs::File::create(dir.join("build.log")).ok();
    }
}

impl Ctx {
    pub(crate) fn log(&self, stage: &str, percent: Option<f32>, level: &str, line: &str) {
        if !line.is_empty() {
            if let Ok(mut log) = LOG.lock() {
                if let Some(file) = log.as_mut() {
                    use std::io::Write;
                    let _ = writeln!(file, "{line}");
                }
            }
        }
        let mut s = state().lock().unwrap();
        s.stage = stage.to_string();
        if let Some(p) = percent {
            // The bar never goes back.
            if p > s.percent {
                s.percent = p;
            }
        }
        if !line.is_empty() {
            s.lines.push(if level == "info" { line.to_string() } else { format!("[{level}] {line}") });
            if s.lines.len() > MAX_LINES {
                let extra = s.lines.len() - MAX_LINES;
                s.lines.drain(..extra);
            }
        }
        let event = Event { stage: stage.to_string(), percent: s.percent, level: level.to_string(), line: line.to_string() };
        drop(s);
        let _ = self.app.emit("build-event", event);
    }

    pub(crate) fn info(&self, stage: &str, percent: f32, line: &str) {
        self.log(stage, Some(percent), "info", line);
    }

    pub(crate) fn level(&self, stage: &str, level: Level, line: &str) {
        self.log(stage, None, if level == Level::Warn { "warn" } else { "info" }, line);
    }
}

pub(crate) fn cancelled() -> bool {
    CANCEL.load(Ordering::Relaxed)
}

pub(crate) fn check_cancel() -> Result<(), String> {
    if cancelled() {
        Err("cancelled".to_string())
    } else {
        Ok(())
    }
}

pub fn build_dir() -> PathBuf {
    PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join("kernel_build")
}

pub fn keys_file() -> PathBuf {
    PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join(".config/soplos-kernel-installer/keys/MOK_combined.pem")
}

// ── Pure helpers ─────────────────────────────────────────────────────────────

/// The percentage of a line of the progress bar of curl: `###### 45.2%`.
pub fn percent_of(line: &str) -> Option<f32> {
    let token = line.split_whitespace().rev().find(|t| t.ends_with('%'))?;
    token.trim_end_matches('%').parse::<f32>().ok().filter(|p| (0.0..=100.0).contains(p))
}

/// The hash that kernel.org lists for a file in its `sha256sums.asc`.
pub fn expected_sha(sums: &str, file: &str) -> Option<String> {
    sums.lines().find_map(|l| {
        let mut parts = l.split_whitespace();
        let (hash, name) = (parts.next()?, parts.next()?);
        (name == file && hash.len() == 64 && hash.chars().all(|c| c.is_ascii_hexdigit())).then(|| hash.to_lowercase())
    })
}

pub fn tarball_url(version: &str) -> String {
    let major = version.split('.').next().unwrap_or("");
    if version.contains("-rc") {
        format!("https://git.kernel.org/torvalds/t/linux-{version}.tar.gz")
    } else {
        format!("https://cdn.kernel.org/pub/linux/kernel/v{major}.x/linux-{version}.tar.xz")
    }
}

pub fn tarball_name(version: &str) -> String {
    format!("linux-{version}.{}", if version.contains("-rc") { "tar.gz" } else { "tar.xz" })
}

/// The file of the newest `linux-headers-*-amd64` package in the index of Debian.
pub fn headers_filename(index: &str) -> Option<String> {
    for block in index.split("\n\n") {
        let field = |name: &str| block.lines().find_map(|l| l.strip_prefix(name).map(|v| v.trim().to_string()));
        let (Some(package), Some(file)) = (field("Package:"), field("Filename:")) else { continue };
        let Some(rest) = package.strip_prefix("linux-headers-") else { continue };
        if rest.starts_with(|c: char| c.is_ascii_digit()) && rest.ends_with("-amd64") && rest.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '+' | '~' | '-')) {
            return Some(file);
        }
    }
    None
}

fn count_c_files(dir: &Path) -> usize {
    fn walk(dir: &Path, n: &mut usize) {
        let Ok(entries) = fs::read_dir(dir) else { return };
        for e in entries.filter_map(|e| e.ok()) {
            let path = e.path();
            if path.is_dir() {
                if e.file_name() != ".git" {
                    walk(&path, n);
                }
            } else if path.extension().map(|x| x == "c").unwrap_or(false) {
                *n += 1;
            }
        }
    }
    let mut n = 0;
    walk(dir, &mut n);
    n
}

// ── Steps ────────────────────────────────────────────────────────────────────

fn sha256_of(file: &Path) -> Option<String> {
    let (code, out) = run_capture({
        let mut c = Command::new("sha256sum");
        c.arg(file);
        c
    });
    (code == 0).then(|| out.split_whitespace().next().unwrap_or("").to_lowercase())
}

fn download_tarball(ctx: &Ctx, version: &str, dest: &Path) -> Result<(), String> {
    let mut cmd = Command::new("curl");
    cmd.args(["-fL", "--retry", "3", "--connect-timeout", "20", "-#", "-o"]).arg(dest).arg(tarball_url(version));
    let mut last = -1.0f32;
    let code = run_stream(cmd, &CANCEL, &mut |line| {
        if let Some(p) = percent_of(line) {
            if p - last >= 1.0 {
                last = p;
                ctx.info("download", 7.0 + p * 0.08, &format!("{}%", p as u32));
            }
        }
    })?;
    check_cancel()?;
    if code != 0 {
        return Err("The kernel could not be downloaded".to_string());
    }
    Ok(())
}

// A tarball that does not match the hash kernel.org publishes is not used.
fn verify_tarball(ctx: &Ctx, version: &str, file: &Path) -> Result<(), String> {
    if version.contains("-rc") {
        ctx.level("verify", Level::Warn, "A release candidate has no published hash: it was not checked");
        return Ok(());
    }
    let major = version.split('.').next().unwrap_or("");
    let (code, sums) = run_capture({
        let mut c = Command::new("curl");
        c.args(["-fsSL", "--connect-timeout", "15", "--max-time", "60", &format!("https://cdn.kernel.org/pub/linux/kernel/v{major}.x/sha256sums.asc")]);
        c
    });
    let name = tarball_name(version);
    let Some(expected) = (code == 0).then(|| expected_sha(&sums, &name)).flatten() else {
        ctx.level("verify", Level::Warn, "The hash of the download could not be read: it was not checked");
        return Ok(());
    };
    let actual = sha256_of(file).ok_or("The hash of the download could not be worked out")?;
    if actual != expected {
        let _ = fs::remove_file(file);
        return Err("The download does not match the hash of kernel.org: it was removed".to_string());
    }
    ctx.info("verify", 15.0, "The download matches the hash of kernel.org");
    Ok(())
}

fn extract(ctx: &Ctx, tarball: &Path, builds: &Path, source: &Path) -> Result<(), String> {
    ctx.info("extract", 16.0, "Extracting the sources");
    let mut cmd = Command::new("tar");
    cmd.arg("-xf").arg(tarball).arg("-C").arg(builds);
    let code = run_stream(cmd, &CANCEL, &mut |_| {})?;
    check_cancel()?;
    if code != 0 || !source.join("Makefile").is_file() {
        return Err("The sources could not be extracted".to_string());
    }
    ctx.info("extract", 25.0, "Sources ready");
    Ok(())
}

fn fetch_debian_config(ctx: &Ctx) -> Option<PathBuf> {
    let tmp = std::env::temp_dir().join(format!("soplos-debian-config-{}", std::process::id()));
    let _ = fs::remove_dir_all(&tmp);
    fs::create_dir_all(&tmp).ok()?;
    let index_xz = tmp.join("Packages.xz");
    if !patches::fetch_file(DEBIAN_PACKAGES, &index_xz) {
        return None;
    }
    let (code, index) = run_capture({
        let mut c = Command::new("xz");
        c.arg("-dc").arg(&index_xz);
        c
    });
    if code != 0 {
        return None;
    }
    let file = headers_filename(&index)?;
    ctx.info("config", 25.0, "Downloading the Debian headers for their configuration");
    let deb = tmp.join("headers.deb");
    if !patches::fetch_file(&format!("{DEBIAN_POOL}{file}"), &deb) {
        return None;
    }
    let out = tmp.join("x");
    let (code, _) = run_capture({
        let mut c = Command::new("dpkg-deb");
        c.arg("-x").arg(&deb).arg(&out);
        c
    });
    if code != 0 {
        return None;
    }
    for e in fs::read_dir(out.join("usr/src")).ok()?.filter_map(|e| e.ok()) {
        let config = e.path().join(".config");
        if e.file_name().to_string_lossy().starts_with("linux-headers-") && config.is_file() {
            return Some(config);
        }
    }
    None
}

/// The broad configuration the build starts from: the current one of Debian,
/// fetched on every build so it never goes stale. If Debian cannot be reached,
/// the last copy; never the configuration of this machine.
fn base_config(ctx: &Ctx, cache: &Path) -> Result<PathBuf, String> {
    if let Some(fresh) = fetch_debian_config(ctx) {
        let _ = fs::copy(&fresh, cache);
        return Ok(fresh);
    }
    if cache.is_file() {
        ctx.level("config", Level::Warn, "Debian could not be reached: using the last copy of its configuration");
        return Ok(cache.to_path_buf());
    }
    Err("No base configuration could be had: Debian could not be reached and there is no earlier copy".to_string())
}

fn multiarch() -> String {
    Command::new("gcc").arg("-print-multiarch").output().map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string()).unwrap_or_default()
}

/// Whether a line of the output of the compilation says what went wrong.
fn is_build_error(line: &str) -> bool {
    let lower = line.to_lowercase();
    lower.contains("error:") || lower.contains("fatal error") || lower.contains("undefined reference") || lower.contains("no rule to make target") || lower.contains("cannot find") || (lower.contains("not found") && lower.contains("***"))
}

fn compile(ctx: &Ctx, source: &Path, cores: u32, kdeb: u32) -> Result<(), String> {
    ctx.info("build", 30.0, &format!("Compiling with {cores} cores"));
    let total = count_c_files(source).max(15000) as f32;
    let mut cmd = Command::new("make");
    cmd.arg(format!("-j{cores}")).arg("bindeb-pkg").current_dir(source).env("KDEB_PKGVERSION", kdeb.to_string());
    // A kernel with debug information (the gaming ones, for sched_ext) would
    // also build a -dbg package of several GB that nobody installs.
    cmd.env("DEB_BUILD_PROFILES", "pkg.linux-upstream.nokerneldbg");
    let arch = multiarch();
    if !arch.is_empty() {
        cmd.env("C_INCLUDE_PATH", format!("/usr/include/{arch}"));
    }
    let mut done = 0f32;
    let mut tail: Vec<String> = Vec::new();
    // The first lines that say what went wrong: the ones at the end of the output
    // are only the `make` of the packages saying that it stopped.
    let mut errors: Vec<String> = Vec::new();
    let code = run_stream(cmd, &CANCEL, &mut |line| {
        let counts = [" CC ", " LD ", " AR "].iter().any(|k| line.contains(k)) || line.starts_with("CC ") || line.starts_with("LD ") || line.starts_with("AR ");
        if counts {
            done += 1.0;
            ctx.info("build", (30.0 + done / total * 60.0).min(90.0), &line.chars().take(90).collect::<String>());
        } else if line.contains("dpkg-deb") || line.contains("dpkg-gencontrol") || line.contains("Wrote:") {
            ctx.info("packages", 90.0, "Making the .deb packages");
        } else {
            ctx.log("build", None, "info", &line.chars().take(120).collect::<String>());
        }
        tail.push(line.to_string());
        if tail.len() > 12 {
            tail.remove(0);
        }
        if errors.len() < 6 && is_build_error(line) {
            errors.push(line.chars().take(200).collect::<String>());
        }
    })?;
    check_cancel()?;
    if code != 0 {
        let last = tail.iter().rev().take(4).rev().cloned().collect::<Vec<_>>().join(" | ");
        return Err(if errors.is_empty() {
            format!("The compilation failed ({code}). Last lines: {last}")
        } else {
            format!("The compilation failed ({code}). First errors: {} | Last lines: {last}", errors.join(" | "))
        });
    }
    Ok(())
}

fn release_of(source: &Path) -> Result<String, String> {
    fs::read_to_string(source.join("include/config/kernel.release")).map(|s| s.trim().to_string()).map_err(|_| "The release of the kernel was not found after the build".to_string())
}

/// Moves the packages of the build to a folder of their own, without the debug ones.
fn isolate_packages(builds: &Path, release: &str) -> Result<PathBuf, String> {
    let target = builds.join(release);
    fs::create_dir_all(&target).map_err(|e| e.to_string())?;
    let mut moved = 0;
    for e in fs::read_dir(builds).map_err(|e| e.to_string())?.filter_map(|e| e.ok()) {
        let name = e.file_name().to_string_lossy().to_string();
        let wanted = name.ends_with(".deb")
            && !name.contains("-dbg")
            && (name.starts_with(&format!("linux-image-{release}_")) || name.starts_with(&format!("linux-headers-{release}_")) || name.starts_with("linux-libc-dev_"));
        if wanted {
            fs::rename(e.path(), target.join(&name)).map_err(|e| e.to_string())?;
            moved += 1;
        }
    }
    if moved < 2 {
        return Err("The packages of the kernel were not found after the build".to_string());
    }
    Ok(target)
}

fn dkms_nvidia(release: &str) -> Option<String> {
    let (_, out) = run_capture({
        let mut c = Command::new("dkms");
        c.arg("status");
        c
    });
    let lines: Vec<&str> = out.lines().filter(|l| l.contains("nvidia") && l.contains(release)).collect();
    if lines.is_empty() {
        return None;
    }
    Some(if lines.iter().any(|l| l.contains("installed")) { "installed" } else { "failed" }.to_string())
}

// ── The job ──────────────────────────────────────────────────────────────────

/// Installs what the build needs, asking for the password once.
pub(crate) fn ensure_deps(ctx: &Ctx) -> Result<(), String> {
    ctx.info("deps", 1.0, "Checking what the build needs");
    let missing = deps::check();
    if !missing.missing.is_empty() || !missing.broken.is_empty() {
        ctx.info("deps", 2.0, &format!("Installing: {}", missing.missing.iter().chain(missing.broken.iter()).cloned().collect::<Vec<_>>().join(" ")));
        let ops = [Op::InstallBuildDeps { packages: missing.missing.clone(), reinstall: missing.broken.clone() }];
        privileged::run_as_root(&ops, |line| ctx.log("deps", None, "info", line)).map_err(|e| if e == "cancelled" { "cancelled".to_string() } else { format!("The tools of the build could not be installed: {e}") })?;
    }
    Ok(())
}

/// From the download to the compiled packages. Returns the release. How long it
/// took goes to the history of times, when the kernel was built whole.
pub(crate) fn build_kernel(ctx: &Ctx, recipe: &Recipe, opts: &Options) -> Result<String, String> {
    let started = std::time::Instant::now();
    let release = build_kernel_inner(ctx, recipe, opts)?;
    super::times::record(&recipe.name, &recipe.version, started.elapsed().as_secs(), recipe.cores);
    Ok(release)
}

fn build_kernel_inner(ctx: &Ctx, recipe: &Recipe, opts: &Options) -> Result<String, String> {
    recipe.validate()?;
    let builds = build_dir();
    fs::create_dir_all(&builds).map_err(|e| format!("{}: {e}", builds.display()))?;
    let source = builds.join(format!("linux-{}", recipe.version));
    let tarball = builds.join(tarball_name(&recipe.version));
    let patch_dir = builds.join("patches");

    // The key is needed before the hours of compiling, not after.
    let signing_key = if recipe.sign {
        let key = keys_file();
        if !key.is_file() {
            return Err("There is no key to sign with: create it in the Secure Boot page first".to_string());
        }
        Some(key)
    } else {
        None
    };

    if opts.check_deps {
        ensure_deps(ctx)?;
    }
    check_cancel()?;

    let mut ids: Vec<String> = recipe.patches.clone();
    // The patch of the ISA level is not a choice: it is needed above the baseline.
    if recipe.march != "v1" {
        ids.push("march".to_string());
    }
    ids.sort();
    ids.dedup();

    // 1. The sources.
    let fresh = if recipe.reuse_sources && source.join("Makefile").is_file() {
        let mut applied = patches::applied(&source);
        applied.sort();
        let mut wanted = ids.clone();
        wanted.retain(|p| p != "ntsync");
        if applied != wanted {
            ctx.info("extract", 2.0, "The patches changed: extracting the sources again");
            tarball.is_file()
        } else {
            ctx.info("extract", 25.0, "Using the sources of the earlier build");
            false
        }
    } else {
        true
    };
    if fresh || !source.join("Makefile").is_file() {
        let cached = (recipe.reuse_sources || opts.keep_tarball) && tarball.is_file() && (recipe.reuse_sources || verify_tarball(ctx, &recipe.version, &tarball).is_ok() && tarball.is_file());
        if !cached {
            for path in [&tarball] {
                let _ = fs::remove_file(path);
            }
            ctx.info("download", 7.0, &format!("Downloading linux-{}", recipe.version));
            download_tarball(ctx, &recipe.version, &tarball)?;
            verify_tarball(ctx, &recipe.version, &tarball)?;
        }
        let _ = fs::remove_dir_all(&source);
        let _ = fs::remove_dir_all(&patch_dir);
        extract(ctx, &tarball, &builds, &source)?;
    }
    check_cancel()?;

    // 2. The patches.
    let to_apply: Vec<String> = ids.iter().filter(|p| *p != "ntsync").cloned().collect();
    if !to_apply.is_empty() && patches::applied(&source).len() != to_apply.len() {
        ctx.info("patches", 20.0, "Downloading the patches");
        let sets = patches::download(&recipe.version, &ids, &patch_dir, &source, &CANCEL, &mut |l, t| ctx.level("patches", l, t))?;
        check_cancel()?;
        ctx.info("patches", 24.0, "Applying the patches");
        patches::apply(&source, &sets, &CANCEL, &mut |l, t| ctx.level("patches", l, t))?;
    }
    check_cancel()?;

    // 3. The configuration.
    ctx.info("config", 25.0, "Fetching the base configuration");
    let base = base_config(ctx, &builds.join("debian-base.config"))?;
    ctx.info("config", 26.0, "Configuring the kernel");
    let hardware = if recipe.goal == "optimized" { recipe::detect_hardware() } else { Vec::new() };
    config::configure(&source, recipe, &base, signing_key.as_deref(), &hardware, &CANCEL, &mut |l, t| ctx.level("config", l, t))?;
    check_cancel()?;
    ctx.info("config", 29.0, "Configuration ready");

    // 4. The build.
    compile(ctx, &source, recipe.cores, opts.kdeb)?;
    release_of(&source)
}

fn run_job(ctx: &Ctx, recipe: &Recipe) -> Result<String, String> {
    reset_log();
    let release = build_kernel(ctx, recipe, &Options::default())?;
    let builds = build_dir();
    let target = isolate_packages(&builds, &release)?;
    ctx.info("packages", 91.0, &format!("Packages of {release} in {}", target.display()));
    check_cancel()?;

    // 5. The install.
    ctx.info("install", 92.0, "Installing the kernel: the password is asked for");
    privileged::run_as_root(&[Op::InstallBuiltKernel { release: release.clone(), rt: recipe.has("rt"), sign: recipe.sign }], |line| ctx.log("install", None, "info", line))
        .map_err(|e| if e == "cancelled" { "cancelled".to_string() } else { e })?;

    let _ = history::save(history::Entry {
        version: release.clone(),
        profile: recipe.goal.clone(),
        patches: if recipe.patches.is_empty() { "none".to_string() } else { recipe.patches.join(", ") },
        installed_date: history::now_iso(),
        secure_boot: recipe.sign,
    });
    if let Ok(mut s) = state().lock() {
        s.nvidia = dkms_nvidia(&release);
    }
    ctx.info("done", 100.0, &format!("{release} is installed"));
    Ok(release)
}

/// Starts work in the background. One at a time.
pub(crate) fn spawn(app: AppHandle, recipe: Option<Recipe>, batch: Option<BatchProgress>, work: impl FnOnce(&Ctx) -> Result<String, String> + Send + 'static) -> Result<(), String> {
    {
        let mut s = state().lock().map_err(|_| "busy".to_string())?;
        if s.running {
            return Err("busy".to_string());
        }
        *s = Snapshot { running: true, stage: "deps".into(), recipe, batch, ..Snapshot::default() };
    }
    CANCEL.store(false, Ordering::Relaxed);
    std::thread::spawn(move || {
        let ctx = Ctx { app: app.clone() };
        let result = work(&ctx);
        let (name, error, release) = match result {
            Ok(release) => ("done", None, Some(release)),
            Err(e) if e == "cancelled" => ("cancelled", None, None),
            Err(e) => ("failed", Some(e), None),
        };
        if let Ok(mut s) = state().lock() {
            s.running = false;
            s.result = Some(name.to_string());
            s.error = error.clone();
            s.release = release;
        }
        let _ = app.emit("build-event", Event { stage: name.to_string(), percent: snapshot().percent, level: if error.is_some() { "error".into() } else { "info".into() }, line: error.unwrap_or_default() });
    });
    Ok(())
}

/// Starts a build in the background.
pub fn start(app: AppHandle, recipe: Recipe) -> Result<(), String> {
    recipe.validate()?;
    let work = recipe.clone();
    spawn(app, Some(recipe), None, move |ctx| run_job(ctx, &work))
}

pub fn cancel() {
    CANCEL.store(true, Ordering::Relaxed);
}

// ── Space ────────────────────────────────────────────────────────────────────

fn dir_size(dir: &Path) -> u64 {
    let Ok(entries) = fs::read_dir(dir) else { return 0 };
    entries
        .filter_map(|e| e.ok())
        .map(|e| {
            let path = e.path();
            match e.file_type() {
                Ok(t) if t.is_dir() => dir_size(&path),
                Ok(t) if t.is_file() => e.metadata().map(|m| m.len()).unwrap_or(0),
                _ => 0,
            }
        })
        .sum()
}

/// The sources and the download of a version: what a clean-up would free.
pub fn files_of(version: &str) -> Result<(PathBuf, PathBuf), String> {
    if !recipe::valid_version(version) {
        return Err(format!("Not a kernel version: {version}"));
    }
    let builds = build_dir();
    Ok((builds.join(format!("linux-{version}")), builds.join(tarball_name(version))))
}

/// Deletes the whole folder of the builds of the user, as the 2.x program did
/// when a build was cancelled and the user did not want to keep it. It only ever
/// removes `~/kernel_build` itself: any other folder is left alone. Returns what
/// it freed. It refuses while a build runs.
pub fn remove_build_dir() -> Result<u64, String> {
    if snapshot().running {
        return Err("busy".to_string());
    }
    let dir = build_dir();
    let home = PathBuf::from(std::env::var_os("HOME").unwrap_or_default());
    if home.as_os_str().is_empty() || dir != home.join("kernel_build") || !dir.is_dir() {
        return Ok(0);
    }
    let freed = dir_size(&dir);
    fs::remove_dir_all(&dir).map_err(|e| e.to_string())?;
    Ok(freed)
}

pub fn files_size(version: &str) -> u64 {
    let Ok((source, tarball)) = files_of(version) else { return 0 };
    dir_size(&source) + fs::metadata(&tarball).map(|m| m.len()).unwrap_or(0)
}

pub fn clean_files(version: &str) -> Result<u64, String> {
    if snapshot().running {
        return Err("busy".to_string());
    }
    let freed = files_size(version);
    let (source, tarball) = files_of(version)?;
    if source.is_dir() {
        fs::remove_dir_all(&source).map_err(|e| e.to_string())?;
    }
    if tarball.is_file() {
        fs::remove_file(&tarball).map_err(|e| e.to_string())?;
    }
    Ok(freed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_progress_of_curl() {
        assert_eq!(percent_of("######## 45.2%"), Some(45.2));
        assert_eq!(percent_of("100.0%"), Some(100.0));
        assert_eq!(percent_of("  0.0%"), Some(0.0));
        assert_eq!(percent_of("curl: (22) error"), None);
        assert_eq!(percent_of("500%"), None);
    }

    #[test]
    fn the_lines_that_say_what_went_wrong() {
        assert!(is_build_error("net/ipv4/tcp_bbr.c:1234:5: error: 'struct tcp_sock' has no member named 'x'"));
        assert!(is_build_error("fatal error: linux/foo.h: No such file or directory"));
        assert!(is_build_error("ld: net/built-in.a: undefined reference to `bbr_init'"));
        assert!(is_build_error("make[4]: *** No rule to make target 'x.o'.  Stop."));
        assert!(!is_build_error("make[2]: *** [scripts/Makefile.package:126: bindeb-pkg] Error 2"));
        assert!(!is_build_error("  CC      kernel/sched/core.o"));
    }

    #[test]
    fn the_hash_of_a_file() {
        let sums = "-----BEGIN PGP SIGNED MESSAGE-----\nHash: SHA256\n\n0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef  linux-7.3.2.tar.xz\nfedcba9876543210fedcba9876543210fedcba9876543210fedcba9876543210  linux-7.3.1.tar.xz\n";
        assert_eq!(expected_sha(sums, "linux-7.3.2.tar.xz").as_deref(), Some("0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"));
        assert_eq!(expected_sha(sums, "linux-7.3.3.tar.xz"), None);
        assert_eq!(expected_sha("short  linux-7.3.2.tar.xz", "linux-7.3.2.tar.xz"), None);
    }

    #[test]
    fn where_a_kernel_comes_from() {
        assert_eq!(tarball_url("7.3.2"), "https://cdn.kernel.org/pub/linux/kernel/v7.x/linux-7.3.2.tar.xz");
        assert_eq!(tarball_url("6.18.31"), "https://cdn.kernel.org/pub/linux/kernel/v6.x/linux-6.18.31.tar.xz");
        assert_eq!(tarball_url("7.4-rc2"), "https://git.kernel.org/torvalds/t/linux-7.4-rc2.tar.gz");
        assert_eq!(tarball_name("7.4-rc2"), "linux-7.4-rc2.tar.gz");
        assert_eq!(tarball_name("7.3.2"), "linux-7.3.2.tar.xz");
    }

    #[test]
    fn the_headers_of_debian() {
        let index = "Package: linux-headers-amd64\nFilename: pool/a.deb\n\nPackage: linux-headers-6.19.2-common\nFilename: pool/b.deb\n\nPackage: linux-headers-6.19.2-amd64\nFilename: pool/main/l/linux/linux-headers-6.19.2-amd64_6.19.2-1_amd64.deb\n\nPackage: other\nFilename: pool/c.deb\n";
        assert_eq!(headers_filename(index).as_deref(), Some("pool/main/l/linux/linux-headers-6.19.2-amd64_6.19.2-1_amd64.deb"));
        assert_eq!(headers_filename("Package: x\nFilename: y\n"), None);
    }

    #[test]
    fn the_packages_are_gathered_without_the_debug_ones() {
        let dir = std::env::temp_dir().join(format!("ki-pkgs-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        for name in ["linux-image-7.3.2-soplos_1_amd64.deb", "linux-image-7.3.2-soplos-dbg_1_amd64.deb", "linux-headers-7.3.2-soplos_1_amd64.deb", "linux-libc-dev_7.3.2-1_amd64.deb", "other.deb"] {
            fs::write(dir.join(name), "").unwrap();
        }
        let target = isolate_packages(&dir, "7.3.2-soplos").unwrap();
        let mut names: Vec<String> = fs::read_dir(&target).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().to_string()).collect();
        names.sort();
        assert_eq!(names, vec!["linux-headers-7.3.2-soplos_1_amd64.deb", "linux-image-7.3.2-soplos_1_amd64.deb", "linux-libc-dev_7.3.2-1_amd64.deb"]);
        assert!(dir.join("linux-image-7.3.2-soplos-dbg_1_amd64.deb").is_file());
        let _ = fs::remove_dir_all(&dir);
    }

    // The two that follow need the network: cargo test -- --ignored
    #[test]
    #[ignore]
    fn the_real_hashes_of_kernel_org_are_read() {
        use crate::build::versions;
        let list = versions::fetch().expect("kernel.org");
        let stable = list.iter().find(|k| k.channel == "stable").expect("a stable kernel");
        let major = stable.version.split('.').next().unwrap();
        let (code, sums) = run_capture({
            let mut c = Command::new("curl");
            c.args(["-fsSL", &format!("https://cdn.kernel.org/pub/linux/kernel/v{major}.x/sha256sums.asc")]);
            c
        });
        assert_eq!(code, 0);
        assert!(expected_sha(&sums, &tarball_name(&stable.version)).is_some(), "no hash for {}", stable.version);
    }

    #[test]
    #[ignore]
    fn the_real_index_of_debian_has_the_headers() {
        let dir = std::env::temp_dir().join(format!("ki-index-{}", std::process::id()));
        let _ = fs::create_dir_all(&dir);
        let xz = dir.join("Packages.xz");
        assert!(patches::fetch_file(DEBIAN_PACKAGES, &xz));
        let (code, index) = run_capture({
            let mut c = Command::new("xz");
            c.arg("-dc").arg(&xz);
            c
        });
        assert_eq!(code, 0);
        let file = headers_filename(&index).expect("linux-headers-*-amd64");
        assert!(file.starts_with("pool/") && file.ends_with(".deb"), "{file}");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn only_a_version_names_a_folder() {
        assert!(files_of("7.3.2").is_ok());
        assert!(files_of("../x").is_err());
    }
}
