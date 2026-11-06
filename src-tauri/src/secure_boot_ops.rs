// What Secure Boot needs the administrator for: telling the firmware which
// key to trust, and signing the kernels and bootloaders with the key of the
// user. The password of the firmware comes through the standard input.

use crate::bootloader;
use crate::privileged::{home_of, valid_release};
use crate::signer::{signer_of, Signer};
use std::fs;
use std::io::{BufRead, Write};
use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

const DKMS_KEY: &str = "/var/lib/dkms/mok.pub";

/// Where root keeps its own copy of the key, for the signing that follows every
/// update. Only root can read it.
pub const ROOT_DIR: &str = "/var/lib/soplos-kernel-installer";
pub const HOOK_FILE: &str = "/etc/apt/apt.conf.d/99soplos-kernel-installer-sign";
const HOOK_BINARY: &str = "/usr/bin/soplos-kernel-installer";

/// The name of the certificate on the boot partition.
pub const CERTIFICATE_FILE: &str = "soplos-secure-boot.cer";

fn env(cmd: &mut Command) -> &mut Command {
    cmd.env("LC_ALL", "C").env("DEBIAN_FRONTEND", "noninteractive").env("PATH", "/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin")
}

fn flush() {
    let _ = std::io::stdout().flush();
}

pub fn key_dir(uid: u32) -> Result<PathBuf, String> {
    Ok(home_of(uid).ok_or("Unknown user")?.join(".config").join("soplos-kernel-installer").join("keys"))
}

// A key file of the user who asked: a plain file of theirs, not a link.
pub(crate) fn user_file(uid: u32, name: &str) -> Result<PathBuf, String> {
    let path = key_dir(uid)?.join(name);
    let meta = fs::symlink_metadata(&path).map_err(|_| format!("The key {name} does not exist"))?;
    if !meta.is_file() || meta.uid() != uid {
        return Err(format!("The key {name} is not valid"));
    }
    Ok(path)
}

fn secret_text(secret: &mut Option<String>) -> Result<String, String> {
    if secret.is_none() {
        let mut line = String::new();
        std::io::stdin().lock().read_line(&mut line).map_err(|e| e.to_string())?;
        *secret = Some(line.trim_end_matches(['\n', '\r']).to_string());
    }
    let text = secret.clone().unwrap_or_default();
    if text.chars().count() < 8 {
        return Err("The password must have at least 8 characters".to_string());
    }
    Ok(text)
}

// mokutil asks for the password twice.
fn mokutil_with_password(args: &[&str], password: &str) -> Result<(), String> {
    let mut child = env(Command::new("mokutil").args(args))
        .stdin(Stdio::piped())
        .spawn()
        .map_err(|e| format!("mokutil: {e}"))?;
    if let Some(mut stdin) = child.stdin.take() {
        let _ = stdin.write_all(format!("{password}\n{password}\n").as_bytes());
    }
    if child.wait().map_err(|e| e.to_string())?.success() {
        Ok(())
    } else {
        Err("mokutil failed".to_string())
    }
}

pub fn install_tools() -> Result<(), String> {
    println!("Installing the tools of Secure Boot");
    flush();
    let status = env(Command::new("apt-get").args(["install", "-y", "mokutil", "openssl", "sbsigntool"])).status().map_err(|e| e.to_string())?;
    if status.success() {
        Ok(())
    } else {
        Err("apt-get install failed".to_string())
    }
}

pub fn enroll(kind: &str, uid: u32, secret: &mut Option<String>) -> Result<(), String> {
    let der = match kind {
        "kernel" => user_file(uid, "MOK.der")?,
        "dkms" => {
            let path = PathBuf::from(DKMS_KEY);
            if !path.is_file() {
                return Err("The key of DKMS does not exist".to_string());
            }
            path
        }
        _ => return Err(format!("Unknown key: {kind}")),
    };
    let password = secret_text(secret)?;
    println!("Queueing the key to be enrolled at the next restart");
    flush();
    mokutil_with_password(&["--import", &der.to_string_lossy()], &password)
}

pub fn delete(uid: u32, secret: &mut Option<String>) -> Result<(), String> {
    let der = user_file(uid, "MOK.der")?;
    let password = secret_text(secret)?;
    println!("Queueing the key to be removed at the next restart");
    flush();
    mokutil_with_password(&["--delete", &der.to_string_lossy()], &password)
}

pub fn reset(secret: &mut Option<String>) -> Result<(), String> {
    let password = secret_text(secret)?;
    println!("Queueing the removal of every enrolled key");
    flush();
    mokutil_with_password(&["--reset"], &password)
}

pub(crate) fn sign(key: &Path, cert: &Path, file: &Path) -> Result<(), String> {
    // A file the firmware already trusts is left as it is: Debian signs its
    // own kernels and bootloader, and a second signature would add nothing.
    if signer_of(file, Some(cert)).is_trusted() {
        println!("{} is already signed", file.display());
        flush();
        return Ok(());
    }
    println!("Signing {}", file.display());
    flush();
    let status = env(Command::new("sbsign").arg("--key").arg(key).arg("--cert").arg(cert).arg("--output").arg(file).arg(file))
        .status()
        .map_err(|e| format!("sbsign: {e}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("Could not sign {}", file.display()))
    }
}

/// The key and the certificate to sign with: the copy root keeps for the
/// automatic signing, or else the ones of the user who asked.
pub(crate) fn signing_keys(uid: u32) -> Result<(PathBuf, PathBuf), String> {
    let root = Path::new(ROOT_DIR);
    if root.join("MOK.priv").is_file() && root.join("MOK.pem").is_file() {
        return Ok((root.join("MOK.priv"), root.join("MOK.pem")));
    }
    Ok((user_file(uid, "MOK.priv")?, user_file(uid, "MOK.pem")?))
}

pub fn sign_kernel(release: &str, uid: u32) -> Result<(), String> {
    if !valid_release(release) {
        return Err(format!("Not a valid kernel release: {release}"));
    }
    let key = user_file(uid, "MOK.priv")?;
    let cert = user_file(uid, "MOK.pem")?;
    let vmlinuz = PathBuf::from(format!("/boot/vmlinuz-{release}"));
    if !vmlinuz.is_file() {
        return Err(format!("{} does not exist", vmlinuz.display()));
    }
    sign(&key, &cert, &vmlinuz)
}

/// Every file the boot of this computer depends on that a key of the user can
/// sign: the kernels of `/boot`, the bootloaders of the boot partition and the
/// kernels and images that systemd-boot loads from it.
pub(crate) fn boot_targets() -> Vec<PathBuf> {
    let mut targets: Vec<PathBuf> = Vec::new();
    if let Ok(dir) = fs::read_dir("/boot") {
        for entry in dir.filter_map(|e| e.ok()) {
            if entry.file_name().to_string_lossy().starts_with("vmlinuz-") {
                targets.push(entry.path());
            }
        }
    }
    for loader in bootloader::detect() {
        targets.extend(loader.files.iter().map(|f| PathBuf::from(&f.path)));
    }
    targets.extend(bootloader::payloads_in(&[Path::new(bootloader::ESP), Path::new("/boot")]));
    targets.sort();
    targets.dedup();
    targets
}

/// Signs what the firmware does not trust yet. A file that is already signed
/// by Debian or with the key is left as it is.
pub fn sign_boot_files(uid: u32) -> Result<(), String> {
    let (key, cert) = signing_keys(uid)?;
    let targets = boot_targets();
    if targets.is_empty() {
        return Err("No kernel or bootloader was found".to_string());
    }
    for file in targets {
        sign(&key, &cert, &file)?;
    }
    Ok(())
}

/// Copies the certificate (the public half) to the boot partition, where the
/// setup of the firmware can pick it up to add it to its own database.
pub fn export_certificate(uid: u32) -> Result<(), String> {
    let der = user_file(uid, "MOK.der")?;
    let dir = Path::new(bootloader::ESP).join("EFI").join("soplos");
    fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let dest = dir.join(CERTIFICATE_FILE);
    fs::copy(&der, &dest).map_err(|e| format!("{}: {e}", dest.display()))?;
    println!("Certificate copied to {}", dest.display());
    flush();
    Ok(())
}

fn hook_text() -> String {
    // After every run of dpkg. It never fails apt: whatever it cannot sign is
    // reported in the Secure Boot page the next time.
    format!("DPkg::Post-Invoke {{ \"{HOOK_BINARY} --sign-boot || true\"; }};\n")
}

fn write_private(path: &Path, bytes: &[u8], mode: u32) -> Result<(), String> {
    let _ = fs::remove_file(path);
    let mut file = fs::OpenOptions::new().write(true).create_new(true).mode(mode).open(path).map_err(|e| format!("{}: {e}", path.display()))?;
    file.write_all(bytes).map_err(|e| e.to_string())
}

/// Copies the key where root can use it and asks apt to sign after each update.
pub fn install_hook(uid: u32) -> Result<(), String> {
    let key = user_file(uid, "MOK.priv")?;
    let cert = user_file(uid, "MOK.pem")?;
    println!("Turning on the automatic signing");
    flush();
    let root = Path::new(ROOT_DIR);
    fs::create_dir_all(root).map_err(|e| e.to_string())?;
    let _ = fs::set_permissions(root, std::os::unix::fs::PermissionsExt::from_mode(0o700));
    write_private(&root.join("MOK.priv"), &fs::read(&key).map_err(|e| e.to_string())?, 0o600)?;
    write_private(&root.join("MOK.pem"), &fs::read(&cert).map_err(|e| e.to_string())?, 0o644)?;
    fs::write(HOOK_FILE, hook_text()).map_err(|e| e.to_string())
}

pub fn remove_hook() -> Result<(), String> {
    println!("Turning off the automatic signing");
    flush();
    for file in [HOOK_FILE, "/var/lib/soplos-kernel-installer/MOK.priv", "/var/lib/soplos-kernel-installer/MOK.pem"] {
        if Path::new(file).exists() {
            fs::remove_file(file).map_err(|e| format!("{file}: {e}"))?;
        }
    }
    let _ = fs::remove_dir(ROOT_DIR);
    Ok(())
}

/// What apt runs after an update, as root and without a window: signs the
/// kernels and the GRUB that were left without any signature. A file signed by
/// someone else is never touched. Always returns 0 so that apt goes on.
pub fn sign_boot() -> i32 {
    let is_root = fs::metadata("/proc/self").map(|m| m.uid() == 0).unwrap_or(false);
    if !is_root {
        eprintln!("This mode is only for root");
        return 2;
    }
    let root = Path::new(ROOT_DIR);
    let (key, cert) = (root.join("MOK.priv"), root.join("MOK.pem"));
    if !key.is_file() || !cert.is_file() {
        return 0;
    }
    // With the hardening of Limine on, the hashes and the binary are made
    // again as a whole, and its binary is not signed on its own.
    let hardened = crate::limine::hardening_on();
    let mut limine_files: Vec<PathBuf> = Vec::new();
    if hardened {
        for loader in bootloader::detect().into_iter().filter(|b| b.id == "limine") {
            limine_files.extend(loader.files.iter().map(|f| PathBuf::from(&f.path)));
        }
        if !limine_files.is_empty() {
            if let Err(e) = crate::limine::refresh(0) {
                eprintln!("{e}");
            }
        }
    }
    for file in boot_targets() {
        if limine_files.contains(&file) {
            continue;
        }
        if signer_of(&file, Some(&cert)) == Signer::None {
            if let Err(e) = sign(&key, &cert, &file) {
                eprintln!("{e}");
            }
        }
    }
    0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_hook_never_fails_apt() {
        let text = hook_text();
        assert!(text.starts_with("DPkg::Post-Invoke {"));
        assert!(text.contains("/usr/bin/soplos-kernel-installer --sign-boot || true"));
        assert!(text.ends_with(";\n"));
    }
}
