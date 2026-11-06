// Which bootloader this computer starts with. Read only: it looks at the files
// of the boot partition and at the entry the firmware used for this start.

use serde::Serialize;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

pub const ESP: &str = "/boot/efi";

#[derive(Serialize, Clone, PartialEq, Eq, Debug)]
#[serde(rename_all = "camelCase")]
pub struct BootFile {
    /// Path on the boot partition, e.g. "/boot/efi/EFI/debian/grubx64.efi".
    pub path: String,
    /// The folder below EFI that holds it, e.g. "debian".
    pub dir: String,
    /// The firmware started the computer through the entry of this folder.
    pub active: bool,
}

#[derive(Serialize, Clone, PartialEq, Eq, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Bootloader {
    /// "grub", "systemd-boot" or "limine".
    pub id: String,
    pub files: Vec<BootFile>,
    /// The firmware started the computer through it this time.
    pub active: bool,
}

/// What kind of bootloader the bytes of an EFI binary are. The name is not
/// enough: shim starts whatever sits next to it under the name of GRUB, so a
/// systemd-boot or a Limine can be found as `grubx64.efi`.
pub fn identify(bytes: &[u8]) -> Option<&'static str> {
    let has = |needle: &[u8]| bytes.windows(needle.len()).any(|w| w == needle);
    if has(b"GNU GRUB") {
        Some("grub")
    } else if has(b"Limine") {
        Some("limine")
    } else if has(b"systemd-boot") {
        Some("systemd-boot")
    } else {
        None
    }
}

/// What kind of bootloader a file name is.
pub fn kind_of(file_name: &str) -> Option<&'static str> {
    let name = file_name.to_lowercase();
    if name == "grubx64.efi" {
        Some("grub")
    } else if name == "systemd-bootx64.efi" {
        Some("systemd-boot")
    } else if name.starts_with("limine") && name.ends_with(".efi") {
        Some("limine")
    } else {
        None
    }
}

/// The folder below EFI the firmware used for this start, read from the lines
/// of `efibootmgr -v`: BootCurrent names the entry, and the entry the file.
pub fn active_dir(efibootmgr: &str) -> Option<String> {
    let current = efibootmgr.lines().find_map(|l| l.strip_prefix("BootCurrent:").map(|v| v.trim().to_uppercase()))?;
    let entry = efibootmgr.lines().find(|l| {
        l.strip_prefix("Boot").map(|rest| rest.to_uppercase().starts_with(&current) && rest[current.len()..].starts_with(['*', ' '])).unwrap_or(false)
    })?;
    let lower = entry.to_lowercase();
    let start = lower.find("file(")? + 5;
    let path = entry[start..].split(')').next()?.replace('\\', "/");
    let mut parts = path.trim_matches('/').split('/');
    if !parts.next()?.eq_ignore_ascii_case("efi") {
        return None;
    }
    parts.next().map(|d| d.to_lowercase())
}

/// The name of the entry the firmware used for this start.
pub fn active_label(efibootmgr: &str) -> Option<String> {
    let current = efibootmgr.lines().find_map(|l| l.strip_prefix("BootCurrent:").map(|v| v.trim().to_uppercase()))?;
    let entry = efibootmgr.lines().find(|l| {
        l.strip_prefix("Boot").map(|rest| rest.to_uppercase().starts_with(&current) && rest[current.len()..].starts_with(['*', ' '])).unwrap_or(false)
    })?;
    let after = entry.get(4 + current.len()..)?.trim_start_matches(['*', ' ']);
    let label = after.split('\t').next()?.trim();
    (!label.is_empty()).then(|| label.to_string())
}

/// The version a binary names after a prefix, e.g. "Limine 12.9.0".
pub fn version_after(bytes: &[u8], prefix: &[u8]) -> Option<String> {
    let at = bytes.windows(prefix.len()).position(|w| w == prefix)? + prefix.len();
    let text: String = bytes[at..].iter().take(24).map(|&b| b as char).take_while(|c| c.is_ascii_digit() || *c == '.' || *c == '-').collect();
    let text = text.trim_matches(['.', '-']).to_string();
    (text.chars().next().map(|c| c.is_ascii_digit()).unwrap_or(false)).then_some(text)
}

pub fn efibootmgr() -> String {
    Command::new("efibootmgr").arg("-v").env("LC_ALL", "C").output().map(|o| String::from_utf8_lossy(&o.stdout).to_string()).unwrap_or_default()
}

/// The bootloaders found on the boot partition, the active one marked.
pub fn detect() -> Vec<Bootloader> {
    detect_in(Path::new(ESP), &efibootmgr())
}

pub fn detect_in(esp: &Path, efibootmgr_text: &str) -> Vec<Bootloader> {
    let active = active_dir(efibootmgr_text);
    let mut found: Vec<Bootloader> = Vec::new();
    let Ok(dirs) = fs::read_dir(esp.join("EFI")) else { return found };
    let mut dirs: Vec<PathBuf> = dirs.filter_map(|d| d.ok()).map(|d| d.path()).filter(|p| p.is_dir()).collect();
    dirs.sort();
    for dir in dirs {
        let dir_name = dir.file_name().map(|n| n.to_string_lossy().to_lowercase()).unwrap_or_default();
        let Ok(files) = fs::read_dir(&dir) else { continue };
        for file in files.filter_map(|f| f.ok()) {
            let name = file.file_name().to_string_lossy().to_string();
            let lower = name.to_lowercase();
            // shim and its helpers are not bootloaders of their own.
            if !lower.ends_with(".efi") || lower.starts_with("shim") || lower.starts_with("mm") || lower.starts_with("fb") {
                continue;
            }
            let by_content = fs::read(file.path()).ok().and_then(|bytes| identify(&bytes));
            let Some(kind) = by_content.or_else(|| kind_of(&name)) else { continue };
            let is_active = active.as_deref() == Some(dir_name.as_str());
            let entry = BootFile { path: file.path().to_string_lossy().to_string(), dir: dir_name.clone(), active: is_active };
            match found.iter_mut().find(|b| b.id == kind) {
                Some(b) => {
                    b.files.push(entry);
                    b.active |= is_active;
                }
                None => found.push(Bootloader { id: kind.to_string(), files: vec![entry], active: is_active }),
            }
        }
    }
    found
}

/// The kernels and images systemd-boot loads: the unified kernel images of
/// `EFI/Linux`, and the `<id>/<version>/linux` files that `kernel-install`
/// copies, on the boot partition and on the extended one.
pub fn payloads_in(roots: &[&Path]) -> Vec<PathBuf> {
    let mut found: Vec<PathBuf> = Vec::new();
    for root in roots {
        if let Ok(files) = fs::read_dir(root.join("EFI").join("Linux")) {
            for f in files.filter_map(|f| f.ok()) {
                if f.file_name().to_string_lossy().to_lowercase().ends_with(".efi") && f.path().is_file() {
                    found.push(f.path());
                }
            }
        }
        let Ok(top) = fs::read_dir(root) else { continue };
        for dir in top.filter_map(|d| d.ok()).map(|d| d.path()).filter(|p| p.is_dir()) {
            let name = dir.file_name().map(|n| n.to_string_lossy().to_lowercase()).unwrap_or_default();
            // Not the folders of the boot partition itself.
            if matches!(name.as_str(), "efi" | "grub" | "loader" | "limine" | "lost+found") {
                continue;
            }
            let Ok(versions) = fs::read_dir(&dir) else { continue };
            for version in versions.filter_map(|v| v.ok()).map(|v| v.path()).filter(|p| p.is_dir()) {
                let linux = version.join("linux");
                if linux.is_file() {
                    found.push(linux);
                }
            }
        }
    }
    found.sort();
    found.dedup();
    found
}

/// Whether shim is the file the firmware started, or sits next to it.
pub fn shim_in(esp: &Path, dir: &str) -> bool {
    esp.join("EFI").join(dir).join("shimx64.efi").is_file()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_names() {
        assert_eq!(kind_of("grubx64.efi"), Some("grub"));
        assert_eq!(kind_of("GRUBX64.EFI"), Some("grub"));
        assert_eq!(kind_of("systemd-bootx64.efi"), Some("systemd-boot"));
        assert_eq!(kind_of("limine_x64.efi"), Some("limine"));
        assert_eq!(kind_of("shimx64.efi"), None);
        assert_eq!(kind_of("BOOTX64.EFI"), None);
    }

    #[test]
    fn the_entry_of_this_start() {
        let text = "BootCurrent: 0003\nTimeout: 1 seconds\nBootOrder: 0003,0001\nBoot0001* UEFI OS\tHD(1,GPT,x)/File(\\EFI\\BOOT\\BOOTX64.EFI)\nBoot0003* debian\tHD(1,GPT,x)/File(\\EFI\\debian\\shimx64.efi)\n";
        assert_eq!(active_dir(text).as_deref(), Some("debian"));
        assert_eq!(active_dir("nothing"), None);
        assert_eq!(active_dir("BootCurrent: 0009\nBoot0003* debian\tHD(1)/File(\\EFI\\debian\\shimx64.efi)\n"), None);
    }

    #[test]
    fn labels_and_versions() {
        let text = "BootCurrent: 0003\nBoot0001* UEFI OS\tHD(1)/File(\\EFI\\BOOT\\BOOTX64.EFI)\nBoot0003* Soplos Linux\tHD(1)/File(\\EFI\\debian\\shimx64.efi)\n";
        assert_eq!(active_label(text).as_deref(), Some("Soplos Linux"));
        assert_eq!(active_label("nothing"), None);
        assert_eq!(version_after(b"xx Limine 12.9.0 yy", b"Limine ").as_deref(), Some("12.9.0"));
        assert_eq!(version_after(b"LoaderInfo systemd-boot 262.2-1 z", b"systemd-boot ").as_deref(), Some("262.2-1"));
        assert_eq!(version_after(b"Limine boot", b"Limine "), None);
    }

    #[test]
    fn the_content_tells_the_bootloader() {
        assert_eq!(identify(b"xx GNU GRUB version 2.14 yy"), Some("grub"));
        assert_eq!(identify(b"abc Limine 12.9.0"), Some("limine"));
        assert_eq!(identify(b"LoaderInfo systemd-boot 262"), Some("systemd-boot"));
        assert_eq!(identify(b"nothing here"), None);
    }

    #[test]
    fn a_systemd_boot_copied_as_grub_is_a_systemd_boot() {
        let root = std::env::temp_dir().join(format!("ki-bootloader-content-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("EFI/debian")).unwrap();
        fs::write(root.join("EFI/debian/grubx64.efi"), b"MZ systemd-boot 262").unwrap();
        fs::write(root.join("EFI/debian/shimx64.efi"), b"MZ shim").unwrap();
        fs::write(root.join("EFI/debian/BOOTX64.EFI"), b"MZ Limine 12.9.0").unwrap();
        let found = detect_in(&root, "");
        let ids: Vec<&str> = found.iter().map(|b| b.id.as_str()).collect();
        assert!(ids.contains(&"systemd-boot"));
        assert!(ids.contains(&"limine"));
        assert!(!ids.contains(&"grub"));
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn payloads_of_systemd_boot() {
        let root = std::env::temp_dir().join(format!("ki-payloads-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("EFI/Linux")).unwrap();
        fs::create_dir_all(root.join("abc123/7.3.1")).unwrap();
        fs::create_dir_all(root.join("grub/7.3.1")).unwrap();
        fs::write(root.join("EFI/Linux/debian-7.3.1.efi"), "").unwrap();
        fs::write(root.join("abc123/7.3.1/linux"), "").unwrap();
        fs::write(root.join("abc123/7.3.1/initrd"), "").unwrap();
        fs::write(root.join("grub/7.3.1/linux"), "").unwrap();
        let found = payloads_in(&[root.as_path()]);
        assert_eq!(found.len(), 2);
        assert!(found.iter().any(|p| p.ends_with("EFI/Linux/debian-7.3.1.efi")));
        assert!(found.iter().any(|p| p.ends_with("abc123/7.3.1/linux")));
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn finds_bootloaders_in_a_folder() {
        let root = std::env::temp_dir().join(format!("ki-bootloader-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("EFI/debian")).unwrap();
        fs::create_dir_all(root.join("EFI/Linux")).unwrap();
        fs::write(root.join("EFI/debian/grubx64.efi"), "").unwrap();
        fs::write(root.join("EFI/debian/shimx64.efi"), "").unwrap();
        let found = detect_in(&root, "BootCurrent: 0001\nBoot0001* debian\tHD(1)/File(\\EFI\\debian\\shimx64.efi)\n");
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].id, "grub");
        assert!(found[0].active);
        assert!(shim_in(&root, "debian"));
        assert!(!shim_in(&root, "Linux"));
        let _ = fs::remove_dir_all(&root);
    }
}
