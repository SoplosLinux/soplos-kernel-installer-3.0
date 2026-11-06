// Limine and Secure Boot. Limine does not use shim: its binary is signed with a
// key the firmware trusts, and it checks the rest of the boot by hash. When the
// hash of its configuration is enrolled in the binary, every file the
// configuration names must carry the BLAKE2B hash of its own, as `path#hash`.
// This reads the configuration, works out those hashes and enrols the new
// configuration in a fresh copy of the binary.

use crate::bootloader;
use crate::secure_boot_ops as sb;
use serde_json::Value;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

pub const MARKER_DIR: &str = "/etc/soplos-kernel-installer";
pub const MARKER: &str = "/etc/soplos-kernel-installer/limine-enrolled";

// ── The configuration ────────────────────────────────────────────────────────

/// A Limine path: `resource(argument):/path#hash`.
#[derive(Debug, PartialEq, Eq, Clone)]
pub struct Resource {
    pub gz: bool,
    pub kind: String,
    pub arg: String,
    pub path: String,
    pub hash: Option<String>,
}

pub fn parse_resource(value: &str) -> Option<Resource> {
    let value = value.trim();
    let (gz, rest) = match value.strip_prefix('$') {
        Some(r) => (true, r),
        None => (false, value),
    };
    let open = rest.find('(')?;
    let close = rest[open..].find(')')? + open;
    let after = rest[close + 1..].strip_prefix(':')?;
    let (path, hash) = split_hash(after);
    Some(Resource { gz, kind: rest[..open].to_string(), arg: rest[open + 1..close].to_string(), path: path.to_string(), hash: hash.map(str::to_string) })
}

// A trailing `#` followed by a long run of hex digits is a hash, not part of the name.
fn split_hash(path: &str) -> (&str, Option<&str>) {
    match path.rfind('#') {
        Some(i) if path.len() - i - 1 >= 32 && path[i + 1..].chars().all(|c| c.is_ascii_hexdigit()) => (&path[..i], Some(&path[i + 1..])),
        _ => (path, None),
    }
}

#[derive(Debug, Default, PartialEq, Eq)]
pub struct Counts {
    pub total: u32,
    /// Has a hash and it is the right one.
    pub matching: u32,
    pub missing: u32,
    pub wrong: u32,
    /// The file could not be found, so its hash could not be worked out.
    pub unresolved: u32,
}

const PATH_KEYS: [&str; 4] = ["path", "kernel_path", "module_path", "dtb_path"];

// The lines that name a file Limine loads and has to check. An entry that
// chainloads another EFI program or a BIOS one is left out: the firmware
// verifies those itself.
fn hashable_lines(text: &str) -> Vec<(usize, String)> {
    let lines: Vec<&str> = text.lines().collect();
    let mut found = Vec::new();
    let mut start = 0;
    while start <= lines.len() {
        // One block: from an entry line up to the next one.
        let end = (start + 1..lines.len()).find(|&i| lines[i].trim_start().starts_with('/')).unwrap_or(lines.len());
        let block = &lines[start.min(lines.len())..end];
        let protocol = block
            .iter()
            .filter_map(|l| l.trim().strip_prefix("protocol:").map(|v| v.trim().to_lowercase()))
            .next()
            .unwrap_or_default();
        let skipped = protocol.starts_with("efi") || protocol.starts_with("bios");
        if !skipped {
            for (offset, line) in block.iter().enumerate() {
                let trimmed = line.trim_start();
                if trimmed.starts_with('#') {
                    continue;
                }
                if let Some((key, value)) = trimmed.split_once(':') {
                    if PATH_KEYS.contains(&key.trim()) && !value.trim().is_empty() {
                        found.push((start + offset, value.trim().to_string()));
                    }
                }
            }
        }
        if end >= lines.len() {
            break;
        }
        start = end;
    }
    found
}

/// Counts how the files of a configuration stand, and, with `write`, gives the
/// text with the right hash on each of them.
pub fn process(text: &str, hash_of: &dyn Fn(&Resource) -> Option<String>, write: bool) -> (String, Counts) {
    let mut counts = Counts::default();
    let mut lines: Vec<String> = text.lines().map(str::to_string).collect();
    for (idx, value) in hashable_lines(text) {
        counts.total += 1;
        let Some(resource) = parse_resource(&value) else {
            counts.unresolved += 1;
            continue;
        };
        let Some(real) = hash_of(&resource) else {
            counts.unresolved += 1;
            continue;
        };
        match &resource.hash {
            Some(h) if h.eq_ignore_ascii_case(&real) => counts.matching += 1,
            Some(_) => counts.wrong += 1,
            None => counts.missing += 1,
        }
        if write {
            let line = &lines[idx];
            let trimmed = line.trim_start();
            let indent = &line[..line.len() - trimmed.len()];
            let key = trimmed.split_once(':').map(|(k, _)| k.trim()).unwrap_or("path");
            let (base, _) = split_hash(value.as_str());
            lines[idx] = format!("{indent}{key}: {base}#{real}");
        }
    }
    let mut out = lines.join("\n");
    if text.ends_with('\n') {
        out.push('\n');
    }
    (out, counts)
}

// ── Which file a path is ─────────────────────────────────────────────────────

#[derive(Debug, Clone, Default)]
pub struct Part {
    pub disk: Option<String>,
    pub number: Option<u32>,
    pub uuid: Option<String>,
    pub partuuid: Option<String>,
    pub mountpoint: Option<String>,
}

pub fn parse_lsblk(json: &str) -> Vec<Part> {
    fn walk(node: &Value, disk: Option<&str>, out: &mut Vec<Part>) {
        let name = node.get("name").and_then(Value::as_str).unwrap_or("").to_string();
        let text = |k: &str| node.get(k).and_then(Value::as_str).map(str::to_string);
        let number = node.get("partn").and_then(|v| v.as_u64().or_else(|| v.as_str().and_then(|s| s.parse().ok()))).map(|n| n as u32);
        let parent = node.get("pkname").and_then(Value::as_str).map(str::to_string).or_else(|| disk.map(str::to_string));
        out.push(Part { disk: parent.clone(), number, uuid: text("uuid"), partuuid: text("partuuid"), mountpoint: text("mountpoint") });
        if let Some(children) = node.get("children").and_then(Value::as_array) {
            for child in children {
                walk(child, Some(&name), out);
            }
        }
    }
    let mut out = Vec::new();
    if let Ok(value) = serde_json::from_str::<Value>(json) {
        if let Some(devices) = value.get("blockdevices").and_then(Value::as_array) {
            for d in devices {
                walk(d, None, &mut out);
            }
        }
    }
    out
}

pub fn load_parts() -> Vec<Part> {
    Command::new("lsblk")
        .args(["-J", "-p", "-o", "NAME,PKNAME,PARTN,UUID,PARTUUID,MOUNTPOINT"])
        .output()
        .map(|o| parse_lsblk(&String::from_utf8_lossy(&o.stdout)))
        .unwrap_or_default()
}

/// Turns the paths of a configuration into files of this system.
pub struct Resolver<'a> {
    pub config_root: &'a Path,
    pub parts: &'a [Part],
}

impl Resolver<'_> {
    fn boot_disk(&self) -> Option<String> {
        let root = self.config_root.to_string_lossy();
        self.parts.iter().find(|p| p.mountpoint.as_deref() == Some(root.as_ref())).and_then(|p| p.disk.clone())
    }

    fn mount(&self, part: &Part) -> Option<PathBuf> {
        part.mountpoint.as_deref().filter(|m| m.starts_with('/')).map(PathBuf::from)
    }

    pub fn resolve(&self, r: &Resource) -> Option<PathBuf> {
        let root = match r.kind.as_str() {
            "boot" if r.arg.is_empty() => self.config_root.to_path_buf(),
            "boot" => {
                let number: u32 = r.arg.parse().ok()?;
                let disk = self.boot_disk()?;
                let part = self.parts.iter().find(|p| p.disk.as_deref() == Some(disk.as_str()) && p.number == Some(number))?;
                self.mount(part)?
            }
            "uuid" | "guid" => {
                let wanted = r.arg.to_lowercase();
                let part = self.parts.iter().find(|p| p.uuid.as_deref().map(str::to_lowercase) == Some(wanted.clone()) || p.partuuid.as_deref().map(str::to_lowercase) == Some(wanted.clone()))?;
                self.mount(part)?
            }
            _ => return None,
        };
        let file = root.join(r.path.trim_start_matches('/'));
        file.is_file().then_some(file)
    }
}

pub fn b2sum(file: &Path) -> Option<String> {
    let out = Command::new("b2sum").arg(file).output().ok()?;
    if !out.status.success() {
        return None;
    }
    String::from_utf8_lossy(&out.stdout).split_whitespace().next().map(str::to_string)
}

/// The configuration Limine reads, in the order its documentation gives: the
/// folder of the program, the fallback folder, then the usual places. Returns
/// the file and the root of the volume it is on.
pub fn find_config(esp: &Path, boot: &Path, app_dir: Option<&str>) -> Option<(PathBuf, PathBuf)> {
    let mut candidates: Vec<(PathBuf, PathBuf)> = Vec::new();
    if let Some(dir) = app_dir {
        candidates.push((esp.join("EFI").join(dir).join("limine.conf"), esp.to_path_buf()));
    }
    candidates.push((esp.join("EFI").join("BOOT").join("limine.conf"), esp.to_path_buf()));
    for root in [esp, boot] {
        for rel in ["boot/limine/limine.conf", "boot/limine.conf", "limine/limine.conf", "limine.conf"] {
            candidates.push((root.join(rel), root.to_path_buf()));
        }
    }
    candidates.into_iter().find(|(file, _)| file.is_file())
}

// ── What the user side reads ─────────────────────────────────────────────────

#[derive(Debug)]
pub struct Audit {
    pub config: Option<PathBuf>,
    pub counts: Counts,
    /// The configuration is the one whose hash this program enrolled.
    pub enrolled: bool,
}

pub fn audit(app_dir: Option<&str>) -> Audit {
    let esp = Path::new(bootloader::ESP);
    let Some((config, root)) = find_config(esp, Path::new("/boot"), app_dir) else {
        return Audit { config: None, counts: Counts::default(), enrolled: false };
    };
    let text = fs::read_to_string(&config).unwrap_or_default();
    let parts = load_parts();
    let resolver = Resolver { config_root: &root, parts: &parts };
    let (_, counts) = process(&text, &|r| resolver.resolve(r).and_then(|f| b2sum(&f)), false);
    let enrolled = fs::read_to_string(MARKER)
        .ok()
        .and_then(|m| m.lines().next().map(str::to_string))
        .map(|recorded| b2sum(&config).as_deref() == Some(recorded.as_str()))
        .unwrap_or(false);
    Audit { config: Some(config), counts, enrolled }
}

// ── What the administrator step does ─────────────────────────────────────────

fn write_atomic(path: &Path, text: &str) -> Result<(), String> {
    let tmp = path.with_extension("conf.new");
    fs::write(&tmp, text).map_err(|e| format!("{}: {e}", tmp.display()))?;
    fs::rename(&tmp, path).map_err(|e| format!("{}: {e}", path.display()))
}

/// Gives every file of the configuration its hash, enrols the hash of the
/// configuration in a fresh copy of the Limine binary, signs it and puts it in
/// place. The copy comes from the files of the package, so it is always
/// enrolled from a clean binary.
pub fn refresh(uid: u32) -> Result<(), String> {
    let (key, cert) = sb::signing_keys(uid)?;
    let loaders = bootloader::detect();
    let limine = loaders.iter().find(|b| b.id == "limine").ok_or("No Limine was found on the boot partition")?;
    let file = limine.files.iter().find(|f| f.active).or_else(|| limine.files.first()).ok_or("No Limine was found on the boot partition")?;
    let target = PathBuf::from(&file.path);
    let (config, root) = find_config(Path::new(bootloader::ESP), Path::new("/boot"), Some(&file.dir)).ok_or("No limine.conf was found")?;

    println!("Working out the hashes of {}", config.display());
    let text = fs::read_to_string(&config).map_err(|e| e.to_string())?;
    let parts = load_parts();
    let resolver = Resolver { config_root: &root, parts: &parts };
    let (new_text, counts) = process(&text, &|r| resolver.resolve(r).and_then(|f| b2sum(&f)), true);
    // A file that cannot be found would stop Limine at the boot: nothing is written.
    if counts.unresolved > 0 {
        return Err(format!("{} files named in the configuration could not be found, so nothing was changed", counts.unresolved));
    }
    if new_text != text {
        write_atomic(&config, &new_text)?;
    }
    let config_hash = b2sum(&config).ok_or("b2sum is not available")?;

    let datadir = Command::new("limine").arg("--print-datadir").output().map_err(|e| format!("limine: {e}"))?;
    let datadir = PathBuf::from(String::from_utf8_lossy(&datadir.stdout).trim());
    let pristine = datadir.join("BOOTX64.EFI");
    if !pristine.is_file() {
        return Err(format!("{} does not exist", pristine.display()));
    }
    println!("Enrolling the configuration in {}", target.display());
    let fresh = target.with_extension("efi.new");
    fs::copy(&pristine, &fresh).map_err(|e| e.to_string())?;
    let enrolled = Command::new("limine").arg("enroll-config").arg(&fresh).arg(&config_hash).status().map_err(|e| e.to_string())?;
    if !enrolled.success() {
        let _ = fs::remove_file(&fresh);
        return Err("limine enroll-config failed".to_string());
    }
    if let Err(e) = sb::sign(&key, &cert, &fresh) {
        let _ = fs::remove_file(&fresh);
        return Err(e);
    }
    fs::rename(&fresh, &target).map_err(|e| e.to_string())?;

    fs::create_dir_all(MARKER_DIR).map_err(|e| e.to_string())?;
    fs::write(MARKER, format!("{config_hash}\n{}\n", target.display())).map_err(|e| e.to_string())
}

/// Whether the user asked for the hardening: the marker is there.
pub fn hardening_on() -> bool {
    Path::new(MARKER).is_file()
}

#[cfg(test)]
mod tests {
    use super::*;

    const HASH: &str = "ca6914d2aabbccddeeff00112233445566778899aabbccddeeff00112233446b470a";

    #[test]
    fn resources() {
        let r = parse_resource("boot(2):/vmlinuz-7.3.1").unwrap();
        assert_eq!((r.kind.as_str(), r.arg.as_str(), r.path.as_str(), r.gz, r.hash.clone()), ("boot", "2", "/vmlinuz-7.3.1", false, None));
        let r = parse_resource(&format!("$boot():/initrd.gz#{HASH}")).unwrap();
        assert!(r.gz);
        assert_eq!(r.arg, "");
        assert_eq!(r.hash.as_deref(), Some(HASH));
        let r = parse_resource("uuid(736b5698-5ae1-4dff-be2c-ef8f44a61c52):/a#b").unwrap();
        assert_eq!(r.path, "/a#b");
        assert_eq!(parse_resource("not a path"), None);
    }

    #[test]
    fn only_the_files_limine_checks_are_hashed() {
        let text = "timeout: 5\n\n/Debian\n    protocol: linux\n    path: boot():/vmlinuz\n    module_path: boot():/initrd\n    cmdline: root=UUID=x\n\n/Windows\n    protocol: efi\n    path: boot():/EFI/Microsoft/bootmgfw.efi\n\n# comment\n/Other\n    path: boot():/k\n";
        let lines = hashable_lines(text);
        let values: Vec<&str> = lines.iter().map(|(_, v)| v.as_str()).collect();
        assert_eq!(values, vec!["boot():/vmlinuz", "boot():/initrd", "boot():/k"]);
    }

    #[test]
    fn hashes_are_written_and_replaced() {
        let text = format!("/Debian\n    protocol: linux\n    path: boot():/vmlinuz\n    module_path: boot():/initrd#{HASH}\n");
        let fake = |r: &Resource| Some(if r.path == "/vmlinuz" { "a".repeat(128) } else { "b".repeat(128) });
        let (count_only, counts) = process(&text, &fake, false);
        assert_eq!(count_only, text);
        assert_eq!((counts.total, counts.missing, counts.wrong, counts.matching), (2, 1, 1, 0));
        let (out, _) = process(&text, &fake, true);
        assert!(out.contains(&format!("path: boot():/vmlinuz#{}", "a".repeat(128))));
        assert!(out.contains(&format!("module_path: boot():/initrd#{}", "b".repeat(128))));
        assert!(out.ends_with('\n'));
        let (_, again) = process(&out, &fake, false);
        assert_eq!((again.total, again.matching), (2, 2));
    }

    #[test]
    fn a_file_that_is_not_found_is_counted() {
        let text = "/A\n    protocol: linux\n    path: boot():/gone\n";
        let (out, counts) = process(text, &|_| None, true);
        assert_eq!(counts.unresolved, 1);
        assert_eq!(out, text);
    }

    #[test]
    fn partitions_of_the_boot_drive() {
        let json = r#"{"blockdevices":[{"name":"/dev/vda","pkname":null,"partn":null,"uuid":null,"partuuid":null,"mountpoint":null,"children":[
            {"name":"/dev/vda1","pkname":"/dev/vda","partn":1,"uuid":"AAAA-1111","partuuid":"p1","mountpoint":"/boot/efi"},
            {"name":"/dev/vda2","pkname":"/dev/vda","partn":2,"uuid":"u2","partuuid":"p2","mountpoint":"/boot"}]}]}"#;
        let parts = parse_lsblk(json);
        let resolver = Resolver { config_root: Path::new("/boot/efi"), parts: &parts };
        assert_eq!(resolver.boot_disk().as_deref(), Some("/dev/vda"));
        let two = parts.iter().find(|p| p.number == Some(2)).unwrap();
        assert_eq!(resolver.mount(two), Some(PathBuf::from("/boot")));
    }

    #[test]
    fn the_configuration_is_found_where_the_documentation_says() {
        let root = std::env::temp_dir().join(format!("ki-limine-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let esp = root.join("esp");
        fs::create_dir_all(esp.join("EFI/limine")).unwrap();
        fs::write(esp.join("limine.conf"), "").unwrap();
        let (file, volume) = find_config(&esp, &root.join("boot"), Some("limine")).unwrap();
        assert_eq!(file, esp.join("limine.conf"));
        assert_eq!(volume, esp);
        fs::write(esp.join("EFI/limine/limine.conf"), "").unwrap();
        assert_eq!(find_config(&esp, &root.join("boot"), Some("limine")).unwrap().0, esp.join("EFI/limine/limine.conf"));
        let _ = fs::remove_dir_all(&root);
    }
}
