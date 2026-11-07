// The last part of a build, which needs the administrator: installing the
// packages, with what NVIDIA needs, and leaving the boot ready. The window never
// sends a script: it names the release, and the packages are found here, in the
// folder of builds of the user who asked.

use super::deps;
use super::nvidia;
use crate::bootloader;
use crate::privileged::{home_of, valid_release};
use crate::secure_boot_ops as sb;
use std::fs;
use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Stdio};

const NVIDIA_RT_CONF: &str = "/etc/modprobe.d/nvidia-drm-rt.conf";

fn say(text: &str) {
    println!("{text}");
    let _ = std::io::stdout().flush();
}

fn env(cmd: &mut Command) -> &mut Command {
    cmd.env("LC_ALL", "C").env("DEBIAN_FRONTEND", "noninteractive").env("PATH", "/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin")
}

fn have(program: &str) -> bool {
    Command::new("sh").args(["-c", &format!("command -v {program}")]).output().map(|o| o.status.success()).unwrap_or(false)
}

pub fn install_deps(packages: &[String], reinstall: &[String]) -> Result<(), String> {
    if let Some(p) = packages.iter().chain(reinstall.iter()).find(|p| !deps::allowed(p)) {
        return Err(format!("Not a package of the build: {p}"));
    }
    say("Installing what the build needs");
    // A repository that cannot be reached must not stop the install of what is
    // already in the lists.
    let _ = env(Command::new("apt-get").args(["update", "-qq"])).status();
    if !packages.is_empty() {
        let status = env(Command::new("apt-get").args(["install", "-y"]).args(packages)).status().map_err(|e| e.to_string())?;
        if !status.success() {
            return Err("apt-get install failed".to_string());
        }
    }
    if !reinstall.is_empty() {
        let status = env(Command::new("apt-get").args(["install", "-y", "--reinstall"]).args(reinstall)).status().map_err(|e| e.to_string())?;
        if !status.success() {
            return Err("apt-get install --reinstall failed".to_string());
        }
    }
    Ok(())
}

// The packages of a release in the folder of builds of the user.
fn packages_of(release: &str, uid: u32) -> Result<Vec<PathBuf>, String> {
    if !valid_release(release) {
        return Err(format!("Not a valid kernel release: {release}"));
    }
    let builds = home_of(uid).ok_or("Unknown user")?.join("kernel_build");
    let dir = builds.join(release).canonicalize().map_err(|_| format!("There is no build of {release}"))?;
    let builds = builds.canonicalize().map_err(|e| e.to_string())?;
    if !dir.starts_with(&builds) || !dir.is_dir() {
        return Err("The packages are not in the folder of builds".to_string());
    }
    let mut debs: Vec<PathBuf> = fs::read_dir(&dir)
        .map_err(|e| e.to_string())?
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| {
            let name = p.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
            name.ends_with(".deb")
                && !name.contains("-dbg")
                && (name.starts_with(&format!("linux-image-{release}_")) || name.starts_with(&format!("linux-headers-{release}_")) || name.starts_with("linux-libc-dev_"))
        })
        .collect();
    debs.sort();
    let has = |prefix: &str| debs.iter().any(|p| p.file_name().map(|n| n.to_string_lossy().starts_with(prefix)).unwrap_or(false));
    if !has(&format!("linux-image-{release}_")) || !has(&format!("linux-headers-{release}_")) {
        return Err(format!("The image and the headers of {release} were not found"));
    }
    Ok(debs)
}

/// Patches the sources of the NVIDIA driver, when the computer has one, before
/// apt builds the module for a new kernel. The patches stay in /usr/src, so the
/// updates that are made later from a terminal build the module too. They skip
/// what is already patched.
pub fn patch_nvidia_sources() {
    if !nvidia::has_nvidia_gpu() {
        return;
    }
    say("Patching the NVIDIA driver for this kernel");
    let status = env(Command::new("sh").arg("-c").arg(nvidia::nvidia_dkms_patch_commands())).stdin(Stdio::null()).status();
    if !matches!(status, Ok(s) if s.success()) {
        say("The patches of the NVIDIA driver did not finish: the module may not build");
    }
}

/// What the package runs after it is installed or updated, as root and without a
/// window: it leaves the sources of the NVIDIA driver patched with the patches
/// this version carries, so the kernels that arrive later from a terminal build
/// the module too. Always returns 0 (but 2 when it is not root) so the
/// installation of the package goes on.
pub fn patch_nvidia_after_install() -> i32 {
    use std::os::unix::fs::MetadataExt;
    let is_root = fs::metadata("/proc/self").map(|m| m.uid() == 0).unwrap_or(false);
    if !is_root {
        eprintln!("This mode is only for root");
        return 2;
    }
    patch_nvidia_sources();
    0
}

/// Installs the packages of a built kernel. `rt` says it is a real-time kernel,
/// which NVIDIA needs to be told about.
pub fn install_built_kernel(release: &str, rt: bool, sign: bool, uid: u32) -> Result<(), String> {
    let debs = packages_of(release, uid)?;
    let nvidia_gpu = nvidia::has_nvidia_gpu();

    patch_nvidia_sources();

    let mut install = Command::new("apt-get");
    install.args(["install", "-y", "--allow-downgrades", "--allow-change-held-packages"]).args(&debs);
    if rt && nvidia_gpu {
        // DKMS refuses to build the NVIDIA module on a real-time kernel unless it
        // is told to, and nvidia-drm needs fbdev=1 to start the display there.
        // Both have to be there before the initramfs is made.
        if !fs::read_to_string(NVIDIA_RT_CONF).map(|t| t.contains("fbdev=1")).unwrap_or(false) {
            fs::write(NVIDIA_RT_CONF, "options nvidia-drm fbdev=1\n").map_err(|e| format!("{NVIDIA_RT_CONF}: {e}"))?;
        }
        let environment = fs::read_to_string("/etc/environment").unwrap_or_default();
        if !environment.contains("IGNORE_PREEMPT_RT_PRESENCE") {
            let mut file = fs::OpenOptions::new().append(true).create(true).open("/etc/environment").map_err(|e| e.to_string())?;
            file.write_all(b"IGNORE_PREEMPT_RT_PRESENCE=1\n").map_err(|e| e.to_string())?;
        }
        install.env("IGNORE_PREEMPT_RT_PRESENCE", "1");
    }
    say("Installing the kernel packages");
    let status = env(&mut install).status().map_err(|e| e.to_string())?;
    if !status.success() {
        if std::path::Path::new(&format!("/boot/vmlinuz-{release}")).exists() {
            // The kernel is there: what failed is the module of a driver.
            say("The kernel is installed, but a module built for it failed (probably the NVIDIA driver). Reinstall the driver from Soplos Welcome");
        } else {
            return Err("The kernel packages could not be installed".to_string());
        }
    }

    if sign {
        say("Signing the kernel");
        sb::sign_kernel(release, uid)?;
    }
    // The boot menu of GRUB lists the kernel by what is in /boot.
    if bootloader::detect().iter().any(|b| b.id == "grub" && b.active) && have("update-grub") {
        say("Updating the boot menu");
        let _ = env(&mut Command::new("update-grub")).status();
    }
    if crate::limine::hardening_on() {
        say("Updating Limine");
        crate::limine::refresh(uid)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_release_that_is_not_one_is_refused() {
        assert!(packages_of("../x", 0).is_err());
        assert!(packages_of("abc", 0).is_err());
    }
}
