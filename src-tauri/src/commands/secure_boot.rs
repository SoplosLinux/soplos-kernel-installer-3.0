// Secure Boot: where the guide stands, the key of the program and the steps
// that need the administrator.

use crate::bootloader::{self, Bootloader};
use crate::history;
use crate::signer::{signer_of, Signer};
use crate::privileged::{self, valid_release, Op};
use serde::Serialize;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use tauri::{AppHandle, Emitter};

const DKMS_KEY: &str = "/var/lib/dkms/mok.pub";

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct KernelSigning {
    pub version: String,
    /// "own", "debian", "microsoft", "other" or "none".
    pub signer: String,
    /// The firmware would let it start: signed by Debian or with the key of the user.
    pub trusted: bool,
    pub is_current: bool,
}

/// A key the firmware trusts, as `mokutil --list-enrolled` lists it.
#[derive(Serialize, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct EnrolledKey {
    pub fingerprint: String,
    pub subject: String,
    /// It is the key of the program.
    pub own: bool,
    /// It is the key DKMS signs its modules with.
    pub dkms: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LoaderFile {
    pub path: String,
    pub dir: String,
    pub active: bool,
    pub signer: String,
    pub trusted: bool,
    /// shim sits in the same folder.
    pub shim: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LoaderState {
    /// "grub", "systemd-boot" or "limine".
    pub id: String,
    pub active: bool,
    pub version: Option<String>,
    pub files: Vec<LoaderFile>,
}

/// The computer the guide is running on.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SystemInfo {
    pub vendor: Option<String>,
    pub product: Option<String>,
    pub cpu: Option<String>,
    pub firmware_vendor: Option<String>,
    pub firmware_version: Option<String>,
    pub firmware_date: Option<String>,
    pub tpm: bool,
    /// The firmware is in setup mode: its keys can be changed without a
    /// signature. None when it cannot be read.
    pub setup_mode: Option<bool>,
    /// The name of the entry the firmware used for this start.
    pub boot_entry: Option<String>,
}

/// A kernel or an image that systemd-boot loads from the boot partition.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PayloadSigning {
    pub path: String,
    pub signer: String,
    pub trusted: bool,
}

/// What Limine needs besides its signature.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LimineState {
    pub config_path: Option<String>,
    /// The files of the configuration, and how many carry the right hash.
    pub total: u32,
    pub matching: u32,
    pub unresolved: u32,
    /// The hash of this configuration is enrolled in the binary.
    pub enrolled: bool,
    /// The user asked for the hardening: the program keeps it up to date.
    pub hardened: bool,
    /// The `limine` tool is installed.
    pub tool: bool,
}

/// One link of the path from the firmware to the kernel.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ChainItem {
    /// "shim", "loader", "kernels", "payloads", "db", "enrolled" or "hashes".
    pub id: String,
    /// "ok", "bad", "unknown" or "off" (not needed or not asked for).
    pub status: String,
    pub count: Option<u32>,
    pub total: Option<u32>,
}

/// The whole path from the firmware to the kernel, for the bootloader that
/// starts this computer.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Chain {
    /// The bootloader that was checked: the active one, or the only one found.
    pub bootloader: Option<String>,
    pub items: Vec<ChainItem>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SecureBootState {
    /// The computer starts in UEFI mode: without it Secure Boot does not exist.
    pub efi: bool,
    pub mokutil: bool,
    pub openssl: bool,
    pub sbsign: bool,
    pub secure_boot_enabled: bool,
    pub keys_exist: bool,
    pub keys_dir: String,
    pub enrolled: bool,
    /// The key was queued and the computer has not restarted since.
    pub enroll_requested: bool,
    pub dkms_modules: bool,
    pub dkms_key_exists: bool,
    pub dkms_enrolled: bool,
    pub dkms_requested: bool,
    pub kernels: Vec<KernelSigning>,
    /// apt signs what an update leaves without a signature.
    pub auto_sign: bool,
    /// The list of keys could be read.
    pub keys_listed: bool,
    pub enrolled_keys: Vec<EnrolledKey>,
    pub system: SystemInfo,
    pub bootloaders: Vec<LoaderState>,
    /// The kernels and images systemd-boot loads from the boot partition.
    pub payloads: Vec<PayloadSigning>,
    pub limine: Option<LimineState>,
    /// The certificate was copied to the boot partition.
    pub cert_exported: bool,
    pub cert_path: String,
    /// The version of GRUB that is installed, e.g. "2.14-5".
    pub grub_version: Option<String>,
    pub chain: Chain,
}

fn keys_dir() -> PathBuf {
    let home = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("/tmp"));
    home.join(".config").join("soplos-kernel-installer").join("keys")
}

fn marker(kind: &str) -> PathBuf {
    history::data_dir().join(format!("mok-requested-{kind}"))
}

fn have(program: &str) -> bool {
    std::env::var_os("PATH")
        .map(|paths| std::env::split_paths(&paths).any(|dir| dir.join(program).is_file()))
        .unwrap_or(false)
        || ["/usr/sbin", "/sbin", "/usr/bin"].iter().any(|d| Path::new(d).join(program).is_file())
}

fn output_of(program: &str, args: &[&std::ffi::OsStr]) -> String {
    Command::new(program)
        .args(args)
        .env("LC_ALL", "C")
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).to_string() + &String::from_utf8_lossy(&o.stderr))
        .unwrap_or_default()
}

fn key_enrolled(der: &Path) -> bool {
    if !der.is_file() || !have("mokutil") {
        return false;
    }
    let text = output_of("mokutil", &["--test-key".as_ref(), der.as_os_str()]).to_lowercase();
    !text.contains("efi variables are not supported") && text.contains("is already enrolled")
}

fn dmi(name: &str) -> Option<String> {
    let text = fs::read_to_string(Path::new("/sys/class/dmi/id").join(name)).ok()?;
    let text = text.trim();
    (!text.is_empty()).then(|| text.to_string())
}

// The variable SetupMode of the firmware: four bytes of attributes, then one.
fn setup_mode() -> Option<bool> {
    let bytes = fs::read("/sys/firmware/efi/efivars/SetupMode-8be4df61-93ca-11d2-aa0d-00e098032b8c").ok()?;
    bytes.last().map(|b| *b == 1)
}

fn system_info(boot_entry: Option<String>) -> SystemInfo {
    SystemInfo {
        vendor: dmi("sys_vendor"),
        product: dmi("product_name"),
        cpu: Some(crate::commands::hardware::cpu_model()).filter(|c| !c.is_empty()),
        firmware_vendor: dmi("bios_vendor"),
        firmware_version: dmi("bios_version"),
        firmware_date: dmi("bios_date"),
        tpm: Path::new("/sys/class/tpm").read_dir().map(|mut d| d.next().is_some()).unwrap_or(false),
        setup_mode: setup_mode(),
        boot_entry,
    }
}

fn loader_version(id: &str, path: &str, grub: Option<&str>) -> Option<String> {
    if id == "grub" {
        return grub.map(str::to_string);
    }
    let bytes = fs::read(path).ok()?;
    let prefix: &[u8] = if id == "limine" { b"Limine " } else { b"systemd-boot " };
    bootloader::version_after(&bytes, prefix)
}

fn has_dkms_modules() -> bool {
    fs::read_dir("/var/lib/dkms")
        .map(|d| d.filter_map(|e| e.ok()).any(|e| e.path().is_dir() && !e.file_name().to_string_lossy().starts_with("mok")))
        .unwrap_or(false)
}

#[tauri::command(async)]
pub fn get_secure_boot_state() -> SecureBootState {
    let dir = keys_dir();
    let der = dir.join("MOK.der");
    let pem = dir.join("MOK.pem");
    let keys_exist = dir.join("MOK.priv").is_file() && der.is_file();
    let mokutil = have("mokutil");
    let sbsign = have("sbsign") && have("sbverify");

    let secure_boot_enabled = mokutil && output_of("mokutil", &["--sb-state".as_ref()]).contains("SecureBoot enabled");
    let enrolled = keys_exist && key_enrolled(&der);
    let dkms_key_exists = Path::new(DKMS_KEY).is_file();
    let dkms_enrolled = dkms_key_exists && key_enrolled(Path::new(DKMS_KEY));

    // A request is forgotten once the key is there.
    if enrolled {
        let _ = fs::remove_file(marker("kernel"));
    }
    if dkms_enrolled {
        let _ = fs::remove_file(marker("dkms"));
    }

    let running = fs::read_to_string("/proc/sys/kernel/osrelease").unwrap_or_default().trim().to_string();
    let own_cert = if keys_exist { Some(pem.as_path()) } else { None };
    let mut kernels: Vec<KernelSigning> = Vec::new();
    if let Ok(dir) = fs::read_dir("/boot") {
        for entry in dir.filter_map(|e| e.ok()) {
            let name = entry.file_name().to_string_lossy().to_string();
            let Some(version) = name.strip_prefix("vmlinuz-") else { continue };
            let signer = if sbsign { signer_of(&entry.path(), own_cert) } else { Signer::None };
            kernels.push(KernelSigning { version: version.to_string(), signer: signer.as_str().to_string(), trusted: signer.is_trusted(), is_current: version == running });
        }
    }
    kernels.sort_by(|a, b| privileged::version_cmp(&b.version, &a.version));

    let esp = Path::new(bootloader::ESP);
    let grub_version = output_of("grub-install", &["--version".as_ref()]).lines().next().and_then(|l| l.rsplit(')').next()).map(|v| v.trim().to_string()).filter(|v| !v.is_empty());
    let efibootmgr_text = bootloader::efibootmgr();
    let bootloaders: Vec<LoaderState> = bootloader::detect()
        .into_iter()
        .map(|b: Bootloader| LoaderState {
            version: b.files.iter().find(|f| f.active).or_else(|| b.files.first()).and_then(|f| loader_version(&b.id, &f.path, grub_version.as_deref())),
            id: b.id,
            active: b.active,
            files: b
                .files
                .into_iter()
                .map(|f| {
                    let signer = if sbsign { signer_of(Path::new(&f.path), own_cert) } else { Signer::None };
                    LoaderFile { shim: bootloader::shim_in(esp, &f.dir), path: f.path, dir: f.dir, active: f.active, signer: signer.as_str().to_string(), trusted: signer.is_trusted() }
                })
                .collect(),
        })
        .collect();
    let payloads: Vec<PayloadSigning> = bootloader::payloads_in(&[esp, Path::new("/boot")])
        .into_iter()
        .map(|path| {
            let signer = if sbsign { signer_of(&path, own_cert) } else { Signer::None };
            PayloadSigning { path: path.to_string_lossy().to_string(), signer: signer.as_str().to_string(), trusted: signer.is_trusted() }
        })
        .collect();

    let limine = bootloaders.iter().find(|l| l.id == "limine").map(|l| {
        let app_dir = l.files.iter().find(|f| f.active).or_else(|| l.files.first()).map(|f| f.dir.clone());
        let audit = crate::limine::audit(app_dir.as_deref());
        LimineState {
            config_path: audit.config.map(|c| c.to_string_lossy().to_string()),
            total: audit.counts.total,
            matching: audit.counts.matching,
            unresolved: audit.counts.unresolved,
            enrolled: audit.enrolled,
            hardened: crate::limine::hardening_on(),
            tool: have("limine"),
        }
    });

    let own_fingerprint = if keys_exist { fingerprint_of(&pem, false) } else { None };
    let dkms_fingerprint = if dkms_key_exists { fingerprint_of(Path::new(DKMS_KEY), true) } else { None };
    let listing = if mokutil { output_of("mokutil", &["--list-enrolled".as_ref()]) } else { String::new() };
    let keys_listed = mokutil && !listing.trim().is_empty() && listing.contains("[key") || (mokutil && listing.contains("MokListRT is empty"));
    // The database of the firmware itself, where the key of a bootloader that does
    // not use shim has to be.
    let db_listing = if mokutil { output_of("mokutil", &["--db".as_ref()]) } else { String::new() };
    let db_keys = parse_enrolled(&db_listing);
    let db_trusted: Option<bool> = match (&own_fingerprint, db_keys.is_empty()) {
        (Some(own), false) => Some(db_keys.iter().any(|(fp, _)| fp == own)),
        (Some(_), true) if db_listing.to_lowercase().contains("empty") => Some(false),
        _ => None,
    };
    let chain = chain_of(&bootloaders, &kernels, &payloads, db_trusted, limine.as_ref());
    let cert_path = Path::new(bootloader::ESP).join("EFI").join("soplos").join(crate::secure_boot_ops::CERTIFICATE_FILE);
    let enrolled_keys: Vec<EnrolledKey> = parse_enrolled(&listing)
        .into_iter()
        .map(|(fingerprint, subject)| EnrolledKey {
            own: own_fingerprint.as_deref() == Some(fingerprint.as_str()),
            dkms: dkms_fingerprint.as_deref() == Some(fingerprint.as_str()),
            fingerprint,
            subject,
        })
        .collect();

    SecureBootState {
        efi: Path::new("/sys/firmware/efi").exists(),
        mokutil,
        openssl: have("openssl"),
        sbsign,
        secure_boot_enabled,
        keys_exist,
        keys_dir: dir.to_string_lossy().to_string(),
        enrolled,
        enroll_requested: !enrolled && marker("kernel").exists(),
        dkms_modules: has_dkms_modules(),
        dkms_key_exists,
        dkms_enrolled,
        dkms_requested: !dkms_enrolled && marker("dkms").exists(),
        kernels,
        auto_sign: Path::new(crate::secure_boot_ops::HOOK_FILE).exists(),
        keys_listed,
        enrolled_keys,
        system: system_info(bootloader::active_label(&efibootmgr_text)),
        bootloaders,
        payloads,
        limine,
        cert_exported: cert_path.is_file(),
        cert_path: cert_path.to_string_lossy().to_string(),
        grub_version,
        chain,
    }
}

/// The SHA1 fingerprint of a certificate, lower case and without colons: the
/// same form `parse_enrolled` gives.
fn fingerprint_of(cert: &Path, der: bool) -> Option<String> {
    let mut cmd = Command::new("openssl");
    cmd.arg("x509").arg("-in").arg(cert);
    if der {
        cmd.args(["-inform", "DER"]);
    }
    let out = cmd.args(["-noout", "-fingerprint", "-sha1"]).output().ok()?;
    let text = String::from_utf8_lossy(&out.stdout).to_string();
    normalize_fingerprint(text.split_once('=')?.1)
}

fn normalize_fingerprint(text: &str) -> Option<String> {
    let clean: String = text.chars().filter(|c| c.is_ascii_hexdigit()).collect::<String>().to_lowercase();
    if clean.len() == 40 {
        Some(clean)
    } else {
        None
    }
}

/// Reads the output of `mokutil --list-enrolled`: for each `[key N]` its SHA1
/// fingerprint and the first subject.
pub fn parse_enrolled(text: &str) -> Vec<(String, String)> {
    let mut keys: Vec<(String, String)> = Vec::new();
    let mut current: Option<(String, String)> = None;
    for line in text.lines() {
        let line = line.trim();
        if line.starts_with("[key") {
            if let Some(key) = current.take() {
                keys.push(key);
            }
            current = Some((String::new(), String::new()));
        } else if let Some(key) = current.as_mut() {
            if let Some(rest) = line.strip_prefix("SHA1 Fingerprint:") {
                key.0 = normalize_fingerprint(rest).unwrap_or_default();
            } else if key.1.is_empty() {
                if let Some(rest) = line.strip_prefix("Subject:") {
                    key.1 = rest.trim().to_string();
                }
            }
        }
    }
    if let Some(key) = current.take() {
        keys.push(key);
    }
    keys
}

fn item(id: &str, status: &str, count: Option<u32>, total: Option<u32>) -> ChainItem {
    ChainItem { id: id.to_string(), status: status.to_string(), count, total }
}

fn status_of(ok: bool) -> &'static str {
    if ok {
        "ok"
    } else {
        "bad"
    }
}

/// The links between the firmware and the kernel, for the bootloader that
/// starts this computer. Each bootloader trusts differently: GRUB and
/// systemd-boot through shim and the key of the user in its list (MOK), or
/// directly through the database of the firmware; Limine only through the
/// database of the firmware, and by hash.
fn chain_of(loaders: &[LoaderState], kernels: &[KernelSigning], payloads: &[PayloadSigning], db_trusted: Option<bool>, limine: Option<&LimineState>) -> Chain {
    // The one that started this computer, or the only one there is.
    let checked = loaders.iter().find(|l| l.active).or_else(|| if loaders.len() == 1 { loaders.first() } else { None });
    let Some(loader) = checked else {
        return Chain { bootloader: None, items: Vec::new() };
    };
    // The files of the entry the firmware used; with none marked, all of them.
    let used: Vec<&LoaderFile> = if loader.files.iter().any(|f| f.active) { loader.files.iter().filter(|f| f.active).collect() } else { loader.files.iter().collect() };
    let loader_trusted = !used.is_empty() && used.iter().all(|f| f.trusted);
    let shim = !used.is_empty() && used.iter().all(|f| f.shim);
    let db = match db_trusted {
        Some(true) => "ok",
        Some(false) => "bad",
        None => "unknown",
    };

    let mut items: Vec<ChainItem> = Vec::new();
    match loader.id.as_str() {
        "grub" => {
            items.push(item("shim", status_of(shim), None, None));
            items.push(item("loader", status_of(loader_trusted), None, None));
            let good = kernels.iter().filter(|k| k.trusted).count() as u32;
            items.push(item("kernels", status_of(good as usize == kernels.len() && !kernels.is_empty()), Some(good), Some(kernels.len() as u32)));
        }
        "systemd-boot" => {
            // Started by shim the key of the user counts through MOK. Started
            // directly, only the database of the firmware can trust it.
            let shim_status = if shim { "ok" } else if db_trusted == Some(true) { "off" } else { "bad" };
            items.push(item("shim", shim_status, None, None));
            items.push(item("loader", status_of(loader_trusted), None, None));
            let good = payloads.iter().filter(|p| p.trusted).count() as u32;
            let status = if payloads.is_empty() { "unknown" } else { status_of(good as usize == payloads.len()) };
            items.push(item("payloads", status, Some(good), Some(payloads.len() as u32)));
            if !shim {
                items.push(item("db", db, None, None));
            }
        }
        "limine" => {
            items.push(item("loader", status_of(loader_trusted), None, None));
            items.push(item("db", db, None, None));
            // Without the hash of the configuration enrolled Limine still boots
            // signed, without checking the rest: the hardening is optional.
            let hardened = limine.map(|l| l.hardened).unwrap_or(false);
            if hardened {
                let l = limine.expect("hardened implies state");
                items.push(item("enrolled", status_of(l.enrolled), None, None));
                let status = if l.unresolved > 0 { "unknown" } else { status_of(l.total > 0 && l.matching == l.total) };
                items.push(item("hashes", status, Some(l.matching), Some(l.total)));
            } else {
                items.push(item("enrolled", "off", None, None));
            }
        }
        _ => {}
    }
    Chain { bootloader: Some(loader.id.clone()), items }
}

fn run(cmd: &mut Command) -> Result<(), String> {
    let out = cmd.output().map_err(|e| e.to_string())?;
    if out.status.success() {
        Ok(())
    } else {
        Err(String::from_utf8_lossy(&out.stderr).trim().to_string())
    }
}

/// Creates the key pair the kernels are signed with. It stays in the folder of
/// the user, readable only by them.
#[tauri::command(async)]
pub fn sb_generate_keys() -> Result<(), String> {
    let dir = keys_dir();
    fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let _ = fs::set_permissions(&dir, fs::Permissions::from_mode(0o700));
    let (priv_key, pem, der, combined) = (dir.join("MOK.priv"), dir.join("MOK.pem"), dir.join("MOK.der"), dir.join("MOK_combined.pem"));

    let base = |cmd: &mut Command| {
        cmd.args(["req", "-new", "-x509", "-newkey", "rsa:2048", "-keyout"])
            .arg(&priv_key)
            .arg("-out")
            .arg(&pem)
            .args(["-nodes", "-days", "36500", "-subj", "/CN=Soplos Kernel Key/"]);
    };
    let mut with_usage = Command::new("openssl");
    base(&mut with_usage);
    with_usage.args(["-addext", "extendedKeyUsage=codeSigning", "-addext", "basicConstraints=critical,CA:FALSE"]);
    if run(&mut with_usage).is_err() {
        // An older openssl does not know -addext.
        let mut plain = Command::new("openssl");
        base(&mut plain);
        run(&mut plain).map_err(|e| format!("openssl: {e}"))?;
    }
    run(Command::new("openssl").args(["x509", "-in"]).arg(&pem).arg("-out").arg(&der).args(["-outform", "DER"]))?;
    // The pair together, as the build of a kernel asks for it.
    let both = fs::read_to_string(&priv_key).map_err(|e| e.to_string())? + &fs::read_to_string(&pem).map_err(|e| e.to_string())?;
    fs::write(&combined, both).map_err(|e| e.to_string())?;
    for secret in [&priv_key, &combined] {
        let _ = fs::set_permissions(secret, fs::Permissions::from_mode(0o600));
    }
    Ok(())
}

fn emit_line(app: &AppHandle) -> impl FnMut(&str) + '_ {
    move |line| {
        let _ = app.emit("ops-line", line.to_string());
    }
}

#[tauri::command(async)]
pub fn sb_install_tools(app: AppHandle) -> Result<(), String> {
    privileged::run_as_root(&[Op::InstallSecureBootTools], emit_line(&app))
}

/// `kind` is "kernel" for the key of the program or "dkms" for the one of DKMS.
#[tauri::command(async)]
pub fn sb_enroll(app: AppHandle, kind: String, password: String) -> Result<(), String> {
    if kind != "kernel" && kind != "dkms" {
        return Err(format!("Unknown key: {kind}"));
    }
    privileged::run_as_root_with_secret(&[Op::EnrollMok { kind: kind.clone() }], Some(&password), emit_line(&app))?;
    let _ = fs::create_dir_all(history::data_dir());
    let _ = fs::write(marker(&kind), "");
    Ok(())
}

/// Queues the key to be removed from the firmware and deletes the local copy.
#[tauri::command(async)]
pub fn sb_delete_key(app: AppHandle, password: String, enrolled: bool) -> Result<(), String> {
    // What needs the administrator, in one password: the key in the firmware, and
    // the copy and the hook that sign after every update.
    let mut ops: Vec<Op> = Vec::new();
    if enrolled {
        ops.push(Op::DeleteMok);
    }
    if Path::new(crate::secure_boot_ops::HOOK_FILE).exists() {
        ops.push(Op::RemoveSigningHook);
    }
    if !ops.is_empty() {
        privileged::run_as_root_with_secret(&ops, Some(&password), emit_line(&app))?;
    }
    let dir = keys_dir();
    for name in ["MOK.priv", "MOK.pem", "MOK.der", "MOK_combined.pem"] {
        let path = dir.join(name);
        if path.exists() {
            fs::remove_file(&path).map_err(|e| e.to_string())?;
        }
    }
    let _ = fs::remove_file(marker("kernel"));
    Ok(())
}

#[tauri::command(async)]
pub fn sb_reset(app: AppHandle, password: String) -> Result<(), String> {
    privileged::run_as_root_with_secret(&[Op::ResetMok], Some(&password), emit_line(&app))?;
    let _ = fs::remove_file(marker("kernel"));
    let _ = fs::remove_file(marker("dkms"));
    Ok(())
}

/// Turns on or off the signing that follows every update.
#[tauri::command(async)]
pub fn sb_set_auto_sign(app: AppHandle, enabled: bool) -> Result<(), String> {
    let op = if enabled { Op::InstallSigningHook } else { Op::RemoveSigningHook };
    privileged::run_as_root(&[op], emit_line(&app))
}

/// Signs what the firmware does not trust yet: bootloaders and kernels.
#[tauri::command(async)]
pub fn sb_sign_boot(app: AppHandle) -> Result<(), String> {
    privileged::run_as_root(&[Op::SignBootFiles], emit_line(&app))
}

/// Limine: gives each file of the configuration its hash and enrols the
/// configuration in a fresh signed copy of the binary.
#[tauri::command(async)]
pub fn sb_limine_refresh(app: AppHandle) -> Result<(), String> {
    privileged::run_as_root(&[Op::LimineRefresh], emit_line(&app))
}

/// Copies the certificate to the boot partition.
#[tauri::command(async)]
pub fn sb_export_cert(app: AppHandle) -> Result<(), String> {
    privileged::run_as_root(&[Op::ExportCertificate], emit_line(&app))
}

/// Signs the kernels of these releases, in one password.
#[tauri::command(async)]
pub fn sb_sign(app: AppHandle, releases: Vec<String>) -> Result<(), String> {
    if releases.is_empty() || releases.iter().any(|r| !valid_release(r)) {
        return Err("Not a valid kernel release".to_string());
    }
    let ops: Vec<Op> = releases.into_iter().map(|release| Op::SignKernel { release }).collect();
    privileged::run_as_root(&ops, emit_line(&app))
}

#[tauri::command(async)]
pub fn reboot_now() -> Result<(), String> {
    run(Command::new("systemctl").arg("reboot"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn enrolled_keys_are_listed() {
        let text = "[key 1]\nSHA1 Fingerprint: 3e:a1:00:11:22:33:44:55:66:77:88:99:aa:bb:cc:dd:ee:ff:01:02\nCertificate:\n    Data:\n        Issuer: CN=Soplos Kernel Key\n        Subject: CN=Soplos Kernel Key\n\n[key 2]\nSHA1 Fingerprint: aa:bb:cc:dd:ee:ff:00:11:22:33:44:55:66:77:88:99:00:11:22:33\nCertificate:\n    Data:\n        Subject: CN=DKMS module signing key\n";
        let keys = parse_enrolled(text);
        assert_eq!(keys.len(), 2);
        assert_eq!(keys[0].0, "3ea1001122334455667788 99aabbccddeeff0102".replace(' ', ""));
        assert_eq!(keys[0].1, "CN=Soplos Kernel Key");
        assert_eq!(keys[1].1, "CN=DKMS module signing key");
        assert!(parse_enrolled("MokListRT is empty").is_empty());
    }

    fn file(dir: &str, signer: &str, shim: bool, active: bool) -> LoaderFile {
        LoaderFile { path: format!("/boot/efi/EFI/{dir}/x.efi"), dir: dir.to_string(), active, signer: signer.to_string(), trusted: signer == "own" || signer == "debian", shim }
    }

    fn loader(id: &str, files: Vec<LoaderFile>) -> LoaderState {
        LoaderState { id: id.to_string(), active: true, version: None, files }
    }

    fn status<'a>(chain: &'a Chain, id: &str) -> Option<&'a str> {
        chain.items.iter().find(|i| i.id == id).map(|i| i.status.as_str())
    }

    #[test]
    fn grub_is_checked_through_shim() {
        let kernels = vec![KernelSigning { version: "7.3".into(), signer: "debian".into(), trusted: true, is_current: true }];
        let ok = chain_of(&[loader("grub", vec![file("debian", "debian", true, true)])], &kernels, &[], None, None);
        assert_eq!(ok.bootloader.as_deref(), Some("grub"));
        assert!(ok.items.iter().all(|i| i.status == "ok"));
        let own = chain_of(&[loader("grub", vec![file("debian", "none", true, true)])], &kernels, &[], None, None);
        assert_eq!(status(&own, "loader"), Some("bad"));
    }

    #[test]
    fn systemd_boot_through_shim_or_directly() {
        let payloads = vec![PayloadSigning { path: "/boot/efi/id/7.3/linux".into(), signer: "own".into(), trusted: true }];
        let via_shim = chain_of(&[loader("systemd-boot", vec![file("debian", "own", true, true)])], &[], &payloads, None, None);
        assert_eq!(status(&via_shim, "shim"), Some("ok"));
        assert_eq!(status(&via_shim, "payloads"), Some("ok"));
        assert_eq!(status(&via_shim, "db"), None);
        let direct = chain_of(&[loader("systemd-boot", vec![file("systemd", "own", false, true)])], &[], &payloads, Some(false), None);
        assert_eq!(status(&direct, "shim"), Some("bad"));
        assert_eq!(status(&direct, "db"), Some("bad"));
        let direct_in_db = chain_of(&[loader("systemd-boot", vec![file("systemd", "own", false, true)])], &[], &payloads, Some(true), None);
        assert_eq!(status(&direct_in_db, "shim"), Some("off"));
        let none = chain_of(&[loader("systemd-boot", vec![file("debian", "own", true, true)])], &[], &[], None, None);
        assert_eq!(status(&none, "payloads"), Some("unknown"));
    }

    #[test]
    fn limine_hardening_is_optional() {
        let plain = LimineState { config_path: None, total: 2, matching: 0, unresolved: 0, enrolled: false, hardened: false, tool: true };
        let chain = chain_of(&[loader("limine", vec![file("limine", "own", false, true)])], &[], &[], Some(true), Some(&plain));
        assert_eq!(status(&chain, "loader"), Some("ok"));
        assert_eq!(status(&chain, "db"), Some("ok"));
        assert_eq!(status(&chain, "enrolled"), Some("off"));
        assert_eq!(status(&chain, "hashes"), None);
        let hardened = LimineState { hardened: true, enrolled: true, matching: 1, ..plain };
        let chain = chain_of(&[loader("limine", vec![file("limine", "own", false, true)])], &[], &[], None, Some(&hardened));
        assert_eq!(status(&chain, "db"), Some("unknown"));
        assert_eq!(status(&chain, "enrolled"), Some("ok"));
        assert_eq!(status(&chain, "hashes"), Some("bad"));
    }

    #[test]
    fn fingerprints_are_comparable() {
        assert_eq!(normalize_fingerprint("3E:A1:00:11:22:33:44:55:66:77:88:99:AA:BB:CC:DD:EE:FF:01:02\n").as_deref(), Some("3ea1001122334455667788 99aabbccddeeff0102".replace(' ', "").as_str()));
        assert_eq!(normalize_fingerprint("not a fingerprint"), None);
    }
}
