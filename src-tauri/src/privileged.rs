// What needs the administrator: the repository of Soplos kernels and the
// packages of the kernels themselves.
//
// The window never runs as root and never sends a script. It writes a request
// with named operations, and pkexec starts this same program again as
// `--run-ops <file>` under the polkit action of the package. That second run
// never opens a window: it checks everything again as root, because the file
// can be written by any user, and only then does the work. Its output is read
// line by line by the window to show the progress.

use crate::kernel_sources as ks;
use crate::secure_boot_ops as sb;
use serde::{Deserialize, Serialize};
use std::fs;
use std::io::{Read, Write};
use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

pub const REPO_FILE: &str = "/etc/apt/sources.list.d/soplos-kernels.sources";
pub const REPO_URL: &str = "https://raw.githubusercontent.com/SoplosLinux/soplos-kernels/main/";
pub const GPG_URL: &str = "https://raw.githubusercontent.com/SoplosLinux/soplos-kernels/main/public.key";
pub const GPG_FILE: &str = "/usr/share/keyrings/soplos-kernels.gpg";

#[derive(Serialize, Deserialize, Clone)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum Op {
    /// Downloads the key, writes the source file and updates the package lists.
    AddKernelRepo,
    /// Removes the source file and the key, then updates the package lists.
    RemoveKernelRepo,
    /// Updates the package lists.
    AptUpdate,
    /// Installs a kernel package of Soplos.
    InstallKernel { package: String },
    /// Removes a kernel package of Soplos with its image and headers.
    RemoveKernel { package: String },
    /// Upgrades a kernel package of Soplos and purges its older versions.
    UpgradeKernel { package: String },
    /// Removes an installed kernel by its release, with every file it left.
    RemoveInstalledKernel { release: String },
    /// Installs or removes a kernel of Debian: linux-image-amd64 or linux-image-rt-amd64.
    InstallDebianKernel { package: String },
    RemoveDebianKernel { package: String },
    /// Installs or removes a variant of XanMod, with its repository.
    InstallXanmod { variant: String },
    RemoveXanmod { variant: String },
    /// Removes a XanMod repository that names a suite XanMod no longer publishes.
    RepairXanmodRepo,
    /// Installs or removes the Liquorix kernel, with its repository.
    InstallLiquorix,
    RemoveLiquorix,
    /// Installs mokutil, openssl and sbsigntool.
    InstallSecureBootTools,
    /// Queues the key of the program ("kernel") or the one of DKMS ("dkms") to
    /// be enrolled in the firmware at the next restart. Needs the password.
    EnrollMok { kind: String },
    /// Queues the key of the program to be removed from the firmware.
    DeleteMok,
    /// Queues the removal of every key enrolled by hand.
    ResetMok,
    /// Signs the kernel of a release and the bootloaders of the boot partition.
    SignKernel { release: String },
    /// Signs what the firmware does not trust yet: the bootloaders and the kernels.
    SignBootFiles,
    /// Gives each file of limine.conf its hash and enrols the configuration.
    LimineRefresh,
    /// Copies the certificate to the boot partition.
    ExportCertificate,
    /// Gives root a copy of the key and signs after every update.
    InstallSigningHook,
    RemoveSigningHook,
    /// Installs what a kernel build needs, and reinstalls what is broken.
    InstallBuildDeps { packages: Vec<String>, reinstall: Vec<String> },
    /// Installs the packages of a kernel that was built in the folder of builds.
    InstallBuiltKernel { release: String, rt: bool, sign: bool },
}

#[derive(Serialize, Deserialize)]
struct Request {
    ops: Vec<Op>,
}

/// The release of a kernel as `uname -r` tells it: "7.3.1-soplos-bore-v4".
pub fn valid_release(release: &str) -> bool {
    let mut chars = release.chars();
    let digits = |s: &str| !s.is_empty() && s.chars().all(|c| c.is_ascii_digit());
    let Some((major, rest)) = release.split_once('.') else {
        return false;
    };
    let minor: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
    let _ = chars.next();
    digits(major) && digits(&minor) && release.len() <= 100 && release.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '+' | '-'))
}

/// The name of a kernel package of Soplos: `linux-soplos`, `linux-soplos-bore-v3`...
pub fn valid_kernel_package(name: &str) -> bool {
    let Some(rest) = name.strip_prefix("linux-soplos") else {
        return false;
    };
    if rest.is_empty() {
        return true;
    }
    let Some(rest) = rest.strip_prefix('-') else {
        return false;
    };
    rest.split('-').all(|part| !part.is_empty() && part.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit()))
}

pub fn cache_root() -> PathBuf {
    let base = std::env::var_os("XDG_CACHE_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".cache")))
        .unwrap_or_else(|| PathBuf::from("/tmp"));
    base.join("soplos-kernel-installer")
}

// ── User side ────────────────────────────────────────────────────────────────

/// Asks pkexec to run the operations as root. One call is one password. Every
/// line the operations print is handed to `on_line`.
pub fn run_as_root(ops: &[Op], on_line: impl FnMut(&str)) -> Result<(), String> {
    run_as_root_with_secret(ops, None, on_line)
}

/// The same, with a secret (a password) that the root side reads from its
/// standard input: it never touches a file and never shows in a command line.
pub fn run_as_root_with_secret(ops: &[Op], secret: Option<&str>, mut on_line: impl FnMut(&str)) -> Result<(), String> {
    let dir = cache_root();
    fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let file = dir.join(format!("request-{}.json", std::process::id()));

    let request = serde_json::to_string(&Request { ops: ops.to_vec() }).map_err(|e| e.to_string())?;
    {
        let mut out = fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .open(&file)
            .map_err(|e| e.to_string())?;
        out.write_all(request.as_bytes()).map_err(|e| e.to_string())?;
    }

    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let spawned = Command::new("pkexec")
        .arg(exe)
        .arg("--run-ops")
        .arg(&file)
        .stdin(if secret.is_some() { Stdio::piped() } else { Stdio::null() })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn();
    let mut child = match spawned {
        Ok(c) => c,
        Err(e) => {
            let _ = fs::remove_file(&file);
            return Err(e.to_string());
        }
    };

    if let Some(secret) = secret {
        if let Some(mut stdin) = child.stdin.take() {
            let _ = stdin.write_all(secret.as_bytes());
            let _ = stdin.write_all(b"\n");
        }
    }

    // The errors of the root side are kept to be shown if it fails.
    let mut stderr = child.stderr.take().expect("piped");
    let errors = std::thread::spawn(move || {
        let mut text = String::new();
        let _ = stderr.read_to_string(&mut text);
        text
    });

    let mut stdout = child.stdout.take().expect("piped");
    read_lines(&mut stdout, &mut on_line);

    let status = child.wait();
    let _ = fs::remove_file(&file);
    let errors = errors.join().unwrap_or_default();

    match status {
        Ok(s) if s.success() => Ok(()),
        // pkexec: 126 is a dismissed password window, 127 a refused one.
        Ok(s) if matches!(s.code(), Some(126) | Some(127)) => Err("cancelled".to_string()),
        Ok(_) => {
            let last = errors.lines().rev().find(|l| !l.trim().is_empty()).unwrap_or("The administrator step did not finish");
            Err(last.trim().to_string())
        }
        Err(e) => Err(e.to_string()),
    }
}

// apt redraws its percentage with a carriage return, so both ends a line.
fn read_lines(reader: &mut impl Read, on_line: &mut impl FnMut(&str)) {
    let mut buf = [0u8; 4096];
    let mut line: Vec<u8> = Vec::new();
    loop {
        match reader.read(&mut buf) {
            Ok(0) | Err(_) => break,
            Ok(n) => {
                for &b in &buf[..n] {
                    if b == b'\n' || b == b'\r' {
                        if !line.is_empty() {
                            on_line(String::from_utf8_lossy(&line).trim());
                            line.clear();
                        }
                    } else {
                        line.push(b);
                    }
                }
            }
        }
    }
    if !line.is_empty() {
        on_line(String::from_utf8_lossy(&line).trim());
    }
}

// ── Root side ────────────────────────────────────────────────────────────────

pub(crate) fn home_of(uid: u32) -> Option<PathBuf> {
    let passwd = fs::read_to_string("/etc/passwd").ok()?;
    passwd.lines().find_map(|line| {
        let fields: Vec<&str> = line.split(':').collect();
        (fields.len() >= 6 && fields[2].parse::<u32>().ok() == Some(uid)).then(|| PathBuf::from(fields[5]))
    })
}

// The cache folder of the user who asked, where the request lives.
fn user_cache(uid: u32) -> Result<PathBuf, String> {
    let home = home_of(uid).ok_or("Unknown user")?;
    home.join(".cache").join("soplos-kernel-installer").canonicalize().map_err(|e| e.to_string())
}

fn apt_env(cmd: &mut Command) -> &mut Command {
    cmd.env("LC_ALL", "C")
        .env("DEBIAN_FRONTEND", "noninteractive")
        .env("PATH", "/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin")
}

fn run(cmd: &mut Command, what: &str) -> Result<(), String> {
    let status = apt_env(cmd).status().map_err(|e| format!("{what}: {e}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("{what} failed"))
    }
}

fn apt_update() -> Result<(), String> {
    run(Command::new("apt-get").arg("update").arg("-qq"), "apt-get update")
}

fn flush() {
    let _ = std::io::stdout().flush();
}

fn add_repo() -> Result<(), String> {
    println!("Adding the Soplos Kernels repository");
    flush();
    let key = apt_env(Command::new("curl").args(["-fsSL", "--max-time", "60", GPG_URL]))
        .stderr(Stdio::inherit())
        .output()
        .map_err(|e| format!("curl: {e}"))?;
    if !key.status.success() || key.stdout.is_empty() {
        return Err("The key of the repository could not be downloaded".to_string());
    }
    let mut gpg = apt_env(Command::new("gpg").args(["--batch", "--yes", "--dearmor", "-o", GPG_FILE]))
        .stdin(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .map_err(|e| format!("gpg: {e}"))?;
    gpg.stdin.take().expect("piped").write_all(&key.stdout).map_err(|e| e.to_string())?;
    if !gpg.wait().map_err(|e| e.to_string())?.success() {
        return Err("The key of the repository is not valid".to_string());
    }
    fs::create_dir_all("/etc/apt/sources.list.d").map_err(|e| e.to_string())?;
    let source = format!("Types: deb\nURIs: {REPO_URL}\nSuites: stable\nComponents: main\nSigned-By: {GPG_FILE}\n");
    fs::write(REPO_FILE, source).map_err(|e| e.to_string())?;
    apt_update()
}

fn remove_repo() -> Result<(), String> {
    println!("Removing the Soplos Kernels repository");
    flush();
    for file in [REPO_FILE, GPG_FILE] {
        if Path::new(file).exists() {
            fs::remove_file(file).map_err(|e| e.to_string())?;
        }
    }
    apt_update()
}

// The part of the release a package owns, e.g. "-soplos-bore-v3" for the
// package linux-soplos-bore-v3: read from what it depends on, as the images
// are named after it.
fn package_suffix(package: &str) -> Result<String, String> {
    let out = apt_env(Command::new("apt-cache").args(["show", package])).output().map_err(|e| e.to_string())?;
    let text = String::from_utf8_lossy(&out.stdout);
    for line in text.lines() {
        if line.is_empty() {
            break;
        }
        if let Some(deps) = line.strip_prefix("Depends:") {
            for dep in deps.split(',') {
                let name = dep.split_whitespace().next().unwrap_or("");
                if name.starts_with("linux-image-") {
                    if let Some(idx) = name.find("-soplos") {
                        return Ok(name[idx..].to_string());
                    }
                }
            }
        }
    }
    Err(format!("{package} is not a kernel of Soplos"))
}

// The installed linux-image and linux-headers packages that end in the suffix.
fn installed_with_suffix(suffix: &str) -> Vec<String> {
    let out = Command::new("dpkg-query").args(["-W", "-f=${db:Status-Abbrev} ${Package}\\n"]).output();
    let Ok(out) = out else { return Vec::new() };
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter_map(|l| {
            let (status, name) = l.split_once(' ')?;
            (status.starts_with("ii") && (name.starts_with("linux-image-") || name.starts_with("linux-headers-")) && name.ends_with(suffix))
                .then(|| name.to_string())
        })
        .collect()
}

/// Orders names the way `sort -V` does: runs of digits as numbers.
pub fn version_cmp(a: &str, b: &str) -> std::cmp::Ordering {
    fn chunks(s: &str) -> Vec<(bool, String)> {
        let mut out: Vec<(bool, String)> = Vec::new();
        for c in s.chars() {
            let digit = c.is_ascii_digit();
            match out.last_mut() {
                Some((d, text)) if *d == digit => text.push(c),
                _ => out.push((digit, c.to_string())),
            }
        }
        out
    }
    let (ca, cb) = (chunks(a), chunks(b));
    for (x, y) in ca.iter().zip(cb.iter()) {
        let ord = if x.0 && y.0 {
            let (nx, ny) = (x.1.trim_start_matches('0'), y.1.trim_start_matches('0'));
            nx.len().cmp(&ny.len()).then_with(|| nx.cmp(ny))
        } else {
            x.1.cmp(&y.1)
        };
        if ord != std::cmp::Ordering::Equal {
            return ord;
        }
    }
    ca.len().cmp(&cb.len())
}

fn install_kernel(package: &str) -> Result<(), String> {
    println!("Installing {package}");
    flush();
    run(Command::new("apt-get").args(["install", "-y", package]), "apt-get install")
}

fn remove_kernel(package: &str) -> Result<(), String> {
    println!("Removing {package}");
    flush();
    let suffix = package_suffix(package)?;
    let mut packages = vec![package.to_string()];
    packages.extend(installed_with_suffix(&suffix));
    // Only the packages of this kernel: a system-wide autoremove would purge
    // packages nobody asked to remove.
    run(Command::new("apt-get").args(["remove", "--purge", "-y"]).args(&packages), "apt-get remove")
}

fn upgrade_kernel(package: &str) -> Result<(), String> {
    println!("Updating {package}");
    flush();
    let suffix = package_suffix(package)?;
    run(Command::new("apt-get").args(["install", "-y", "--only-upgrade", package]), "apt-get upgrade")?;
    // Purge the older images and headers of this kernel: all but the newest,
    // and never the one that is running.
    let running = fs::read_to_string("/proc/sys/kernel/osrelease").unwrap_or_default().trim().to_string();
    for prefix in ["linux-image-", "linux-headers-"] {
        let mut names: Vec<String> = installed_with_suffix(&suffix).into_iter().filter(|n| n.starts_with(prefix)).collect();
        names.sort_by(|a, b| version_cmp(a, b));
        names.pop();
        names.retain(|n| !(prefix == "linux-image-" && n.ends_with(&running)) && !n.contains(&running));
        if !names.is_empty() {
            println!("Purging older versions");
            flush();
            // The old versions going away is a clean-up: the upgrade itself worked.
            let _ = run(Command::new("apt-get").args(["purge", "-y"]).args(&names), "apt-get purge");
        }
    }
    Ok(())
}

fn package_installed(name: &str) -> bool {
    Command::new("dpkg-query")
        .args(["-W", "-f=${db:Status-Abbrev}", name])
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).starts_with("ii"))
        .unwrap_or(false)
}

fn remove_installed_kernel(release: &str) -> Result<(), String> {
    // The kernel that is running is never removed, whatever the window said.
    let running = fs::read_to_string("/proc/sys/kernel/osrelease").unwrap_or_default();
    if running.trim() == release {
        return Err("The kernel that is running cannot be removed".to_string());
    }
    println!("Removing kernel {release}");
    flush();
    // Only the packages that exist: a kernel put there by hand has none.
    let packages: Vec<String> = [format!("linux-image-{release}"), format!("linux-headers-{release}")]
        .into_iter()
        .filter(|p| package_installed(p))
        .collect();
    if !packages.is_empty() {
        // Only this kernel's packages: a system-wide autoremove would purge
        // packages nobody asked to remove.
        run(Command::new("apt-get").args(["remove", "--purge", "-y"]).args(&packages), "apt-get remove")?;
    }
    // What the packages may have left, or a kernel without packages.
    for file in ["vmlinuz", "initrd.img", "System.map", "config"] {
        let path = format!("/boot/{file}-{release}");
        if Path::new(&path).exists() {
            fs::remove_file(&path).map_err(|e| format!("{path}: {e}"))?;
        }
    }
    for dir in [format!("/lib/modules/{release}"), format!("/usr/src/linux-headers-{release}")] {
        if Path::new(&dir).exists() {
            fs::remove_dir_all(&dir).map_err(|e| format!("{dir}: {e}"))?;
        }
    }
    // The modules DKMS built for it. Nothing to do when DKMS is not there.
    if Command::new("sh").args(["-c", "command -v dkms"]).output().map(|o| o.status.success()).unwrap_or(false) {
        let _ = apt_env(Command::new("dkms").args(["remove", "--all", "-k", release])).status();
    }
    // The nvidia-drm setting of the real-time kernels is only needed while one is left.
    let rt_conf = Path::new("/etc/modprobe.d/nvidia-drm-rt.conf");
    if rt_conf.exists() && !any_rt_kernel_left() {
        let _ = fs::remove_file(rt_conf);
    }
    if Command::new("sh").args(["-c", "command -v update-grub"]).output().map(|o| o.status.success()).unwrap_or(false) {
        run(&mut Command::new("update-grub"), "update-grub")?;
    }
    Ok(())
}

fn any_rt_kernel_left() -> bool {
    fs::read_dir("/boot")
        .map(|d| {
            d.filter_map(|e| e.ok())
                .filter(|e| e.file_name().to_string_lossy().starts_with("config-"))
                .any(|e| fs::read_to_string(e.path()).map(|t| t.lines().any(|l| l == "CONFIG_PREEMPT_RT=y")).unwrap_or(false))
        })
        .unwrap_or(false)
}

fn run_op(op: &Op, uid: u32, secret: &mut Option<String>) -> Result<(), String> {
    match op {
        Op::AddKernelRepo => add_repo(),
        Op::RemoveKernelRepo => remove_repo(),
        Op::AptUpdate => apt_update(),
        Op::InstallSecureBootTools => sb::install_tools(),
        Op::EnrollMok { kind } => sb::enroll(kind, uid, secret),
        Op::DeleteMok => sb::delete(uid, secret),
        Op::ResetMok => sb::reset(secret),
        Op::SignKernel { release } => sb::sign_kernel(release, uid),
        Op::SignBootFiles => sb::sign_boot_files(uid),
        Op::LimineRefresh => crate::limine::refresh(uid),
        Op::ExportCertificate => sb::export_certificate(uid),
        Op::InstallBuildDeps { packages, reinstall } => crate::build::install::install_deps(packages, reinstall),
        Op::InstallBuiltKernel { release, rt, sign } => {
            if !valid_release(release) {
                return Err(format!("Not a valid kernel release: {release}"));
            }
            crate::build::install::install_built_kernel(release, *rt, *sign, uid)
        }
        Op::InstallSigningHook => sb::install_hook(uid),
        Op::RemoveSigningHook => sb::remove_hook(),
        Op::InstallDebianKernel { package } => ks::install_debian(package),
        Op::RemoveDebianKernel { package } => ks::remove_debian(package),
        Op::InstallXanmod { variant } => {
            if !ks::XANMOD_VARIANTS.contains(&variant.as_str()) {
                return Err(format!("Not a XanMod variant: {variant}"));
            }
            ks::install_xanmod(variant)
        }
        Op::RemoveXanmod { variant } => {
            if !ks::XANMOD_VARIANTS.contains(&variant.as_str()) {
                return Err(format!("Not a XanMod variant: {variant}"));
            }
            ks::remove_xanmod(variant)
        }
        Op::RepairXanmodRepo => ks::repair_xanmod(),
        Op::InstallLiquorix => ks::install_liquorix(),
        Op::RemoveLiquorix => ks::remove_liquorix(),
        Op::RemoveInstalledKernel { release } => {
            if !valid_release(release) {
                return Err(format!("Not a valid kernel release: {release}"));
            }
            remove_installed_kernel(release)
        }
        Op::InstallKernel { package } | Op::RemoveKernel { package } | Op::UpgradeKernel { package } => {
            if !valid_kernel_package(package) {
                return Err(format!("Not a kernel package of Soplos: {package}"));
            }
            match op {
                Op::InstallKernel { .. } => install_kernel(package),
                Op::RemoveKernel { .. } => remove_kernel(package),
                _ => upgrade_kernel(package),
            }
        }
    }
}

/// Called from main() before Tauri starts when pkexec starts the program again.
/// Returns the exit code.
pub fn run_ops_file(path: &str) -> i32 {
    let uid = match std::env::var("PKEXEC_UID").ok().and_then(|v| v.parse::<u32>().ok()) {
        Some(uid) => uid,
        None => {
            eprintln!("This mode is only for pkexec");
            return 2;
        }
    };
    let result = (|| -> Result<(), String> {
        let cache = user_cache(uid)?;
        let file = Path::new(path).canonicalize().map_err(|e| e.to_string())?;
        let meta = fs::metadata(&file).map_err(|e| e.to_string())?;
        // The request has to be a file of the user who asked, in their cache.
        if !file.starts_with(&cache) || !meta.is_file() || meta.uid() != uid {
            return Err("The request is not valid".to_string());
        }
        let request: Request = serde_json::from_str(&fs::read_to_string(&file).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
        let mut secret: Option<String> = None;
        for op in &request.ops {
            run_op(op, uid, &mut secret)?;
        }
        Ok(())
    })();
    match result {
        Ok(()) => 0,
        Err(e) => {
            eprintln!("{e}");
            1
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cmp::Ordering;

    #[test]
    fn package_names() {
        assert!(valid_kernel_package("linux-soplos"));
        assert!(valid_kernel_package("linux-soplos-bore-v3"));
        assert!(valid_kernel_package("linux-soplos-zen"));
        assert!(!valid_kernel_package("linux-soplosx"));
        assert!(!valid_kernel_package("linux-soplos-"));
        assert!(!valid_kernel_package("linux-soplos--v3"));
        assert!(!valid_kernel_package("linux-soplos-bore v3"));
        assert!(!valid_kernel_package("linux-soplos-Bore"));
        assert!(!valid_kernel_package("linux-soplos;rm"));
        assert!(!valid_kernel_package("evil"));
    }

    #[test]
    fn releases() {
        assert!(valid_release("7.3.1-soplos-bore-ntsync-v4"));
        assert!(valid_release("6.12.9+deb13-amd64"));
        assert!(valid_release("7.3-rc1"));
        assert!(!valid_release("../etc"));
        assert!(!valid_release("7.3/x"));
        assert!(!valid_release("abc"));
        assert!(!valid_release("7"));
        assert!(!valid_release("7.x"));
        assert!(!valid_release("7.3 1"));
    }

    #[test]
    fn versions_sort_like_sort_v() {
        assert_eq!(version_cmp("linux-image-7.3.2-soplos", "linux-image-7.3.10-soplos"), Ordering::Less);
        assert_eq!(version_cmp("linux-image-7.10.0-soplos", "linux-image-7.9.9-soplos"), Ordering::Greater);
        assert_eq!(version_cmp("linux-image-7.3.1-soplos", "linux-image-7.3.1-soplos"), Ordering::Equal);
    }
}
