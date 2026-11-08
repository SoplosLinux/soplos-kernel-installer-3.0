// Configuring the kernel: the base configuration, the profile, the patches and
// the fixes, then olddefconfig and the checks that follow. Ported from
// `SoplosInstaller.configure` of the 2.x program, with two changes the user
// asked for: Rust is part of the kernel and the Binder of Waydroid is the Rust
// one only. A kernel that has no Rust Binder (6.17 and older) is built without
// Binder and without Rust.

use super::recipe::{profile_ops, ConfigOp, Recipe};
use super::runner::{run_capture, run_stream};
use super::versions::major_minor;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::AtomicBool;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Level {
    Info,
    Warn,
}

pub struct Tree {
    pub dir: PathBuf,
}

impl Tree {
    fn run(&self, ops: &[ConfigOp]) {
        for op in ops {
            let _ = Command::new("./scripts/config").args(op.args()).current_dir(&self.dir).output();
        }
    }

    /// "y", "m", "n" or "undef".
    fn state(&self, name: &str) -> String {
        let (_, out) = run_capture({
            let mut c = Command::new("./scripts/config");
            c.args(["--state", name]).current_dir(&self.dir);
            c
        });
        out.trim().to_string()
    }

    fn is_on(&self, name: &str) -> bool {
        self.state(name) == "y"
    }

    /// Whether the Kconfig of the tree declares a symbol.
    fn declares(&self, file: &str, symbol: &str) -> bool {
        fs::read_to_string(self.dir.join(file)).map(|t| t.lines().any(|l| l.trim() == format!("config {symbol}"))).unwrap_or(false)
    }
}

/// Whether the tree has the Rust Binder driver (it came with 6.18).
pub fn has_rust_binder(dir: &Path) -> bool {
    Tree { dir: dir.to_path_buf() }.declares("drivers/android/Kconfig", "ANDROID_BINDER_IPC_RUST")
}

fn debug_info_fixes() -> Vec<ConfigOp> {
    use ConfigOp::*;
    vec![
        Disable("DEBUG_INFO_DWARF_TOOLCHAIN_DEFAULT".into()),
        Disable("DEBUG_INFO_DWARF4".into()),
        Disable("DEBUG_INFO_DWARF5".into()),
        Disable("DEBUG_INFO_BTF".into()),
        Disable("DEBUG_INFO_BTF_MODULES".into()),
        Disable("DEBUG_INFO_REDUCED".into()),
        Disable("DEBUG_INFO_SPLIT".into()),
        Disable("DEBUG_KERNEL".into()),
        Disable("DEBUG_PREEMPT".into()),
        Disable("GDB_SCRIPTS".into()),
        Enable("DEBUG_INFO_NONE".into()),
    ]
}

fn config_fixes(signing_key: Option<&Path>) -> Vec<ConfigOp> {
    use ConfigOp::*;
    let mut fixes = vec![
        SetStr("SYSTEM_TRUSTED_KEYS".into(), String::new()),
        SetStr("SYSTEM_REVOCATION_KEYS".into(), String::new()),
        // The Debian base configuration asks for the QR code panic screen, which
        // needs a Rust support this build may not have: the kernel then logs
        // "Unsupported value" at every start.
        SetStr("DRM_PANIC_SCREEN".into(), "user".into()),
        // With MODVERSIONS, Rust asks for GENDWARFKSYMS, which asks for debug
        // information, which triples the build time.
        Disable("MODVERSIONS".into()),
    ];
    fixes.extend(debug_info_fixes());
    // The embedded controller drivers of the gaming handhelds, missing from the
    // Debian base configuration.
    fixes.push(Module("ASUS_ARMOURY".into()));
    fixes.push(Module("OXP_EC".into()));
    match signing_key {
        Some(key) => {
            fixes.push(Enable("MODULE_SIG".into()));
            fixes.push(Enable("MODULE_SIG_ALL".into()));
            fixes.push(SetStr("MODULE_SIG_KEY".into(), key.to_string_lossy().to_string()));
        }
        None => {
            // The lockdown module selects MODULE_SIG, so without these a kernel
            // would check signatures of modules nobody signed, and taint itself
            // with every one of them (8192).
            fixes.push(Disable("SECURITY_LOCKDOWN_LSM".into()));
            fixes.push(Disable("SECURITY_LOCKDOWN_LSM_EARLY".into()));
            fixes.push(Disable("MODULE_SIG".into()));
            fixes.push(Disable("MODULE_SIG_ALL".into()));
            fixes.push(SetStr("MODULE_SIG_KEY".into(), "certs/signing_key.pem".into()));
        }
    }
    fixes
}

fn march_ops(level: &str) -> (Option<&'static str>, Vec<ConfigOp>) {
    let all = ["X86_64_ISA_V1", "X86_64_ISA_V2", "X86_64_ISA_V3", "X86_64_ISA_V4"];
    let wanted = match level {
        "v2" => "X86_64_ISA_V2",
        "v3" => "X86_64_ISA_V3",
        "v4" => "X86_64_ISA_V4",
        // v1 is the baseline of the kernel and needs no symbol.
        _ => return (None, Vec::new()),
    };
    let mut ops: Vec<ConfigOp> = all.iter().filter(|s| **s != wanted).map(|s| ConfigOp::Disable(s.to_string())).collect();
    ops.push(ConfigOp::Enable(wanted.to_string()));
    (Some(wanted), ops)
}

/// What the patches must have added to the configuration.
fn scheduler_symbol(patch: &str) -> Option<&'static str> {
    match patch {
        "bore" | "x3d" => Some("SCHED_BORE"),
        "zen" => Some("SCHED_ALT"),
        _ => None,
    }
}

pub fn ntsync_in_kernel(version: &str) -> bool {
    major_minor(version).map(|m| m >= (6, 14)).unwrap_or(false)
}

/// Configures the source tree. `emit` gets what the user should read.
pub fn configure(
    dir: &Path,
    recipe: &Recipe,
    base_config: &Path,
    signing_key: Option<&Path>,
    hardware: &[(String, String)],
    cancel: &AtomicBool,
    emit: &mut dyn FnMut(Level, &str),
) -> Result<(), String> {
    let tree = Tree { dir: dir.to_path_buf() };
    fs::copy(base_config, dir.join(".config")).map_err(|e| format!("The base configuration could not be copied: {e}"))?;

    emit(Level::Info, "Applying the profile");
    tree.run(&profile_ops(&recipe.goal, hardware));
    tree.run(&[ConfigOp::SetStr("LOCALVERSION".into(), recipe.local_version())]);

    if recipe.has("ntsync") {
        if ntsync_in_kernel(&recipe.version) {
            tree.run(&[ConfigOp::Enable("NTSYNC".into())]);
        } else {
            emit(Level::Warn, "NTSYNC needs kernel 6.14 or newer: left out");
        }
    }
    if recipe.has("rt") {
        // PREEMPT_RT is independent of the choice of preemption; PREEMPT_DYNAMIC
        // depends on it being off.
        tree.run(&[ConfigOp::Enable("PREEMPT_RT".into()), ConfigOp::Disable("PREEMPT_DYNAMIC".into())]);
    }
    if recipe.has("zen") {
        // The patch exists for its alternate scheduler, which is off by default.
        tree.run(&[ConfigOp::Enable("SCHED_ALT".into()), ConfigOp::Enable("SCHED_PDS".into())]);
    }
    let (march_symbol, march) = march_ops(&recipe.march);
    if !march.is_empty() {
        emit(Level::Info, &format!("Architecture level {}", recipe.march));
        tree.run(&march);
    }
    if recipe.sched_ext {
        tree.run(&[ConfigOp::Enable("SCHED_CLASS_EXT".into())]);
    }
    // Device memory accounting, for the gaming daemon that will come. It is a
    // compile-time option, so it goes in now.
    tree.run(&[ConfigOp::Enable("CGROUP_DMEM".into())]);

    let rust_binder = has_rust_binder(dir);
    if rust_binder {
        emit(Level::Info, "Rust and the Binder of Waydroid");
        if super::deps::build_path().is_some() {
            emit(Level::Info, "The Rust of /usr/bin is used: the one of rustup has no rust-src");
        }
        tree.run(&[ConfigOp::Enable("RUST".into())]);
        // The Rust driver depends on the C one being off.
        if tree.declares("drivers/android/Kconfig", "ANDROID_BINDER_IPC") {
            tree.run(&[ConfigOp::Disable("ANDROID_BINDER_IPC".into())]);
        }
        tree.run(&[
            ConfigOp::Enable("ANDROID_BINDER_IPC_RUST".into()),
            ConfigOp::SetStr("ANDROID_BINDER_DEVICES".into(), "binder,hwbinder,vndbinder".into()),
        ]);
    } else {
        emit(Level::Warn, "This kernel has no Rust Binder: it is built without Binder and without Rust, and Waydroid will not work with it");
        // The base configuration of Debian turns Rust on, and an old kernel may
        // not build with the toolchain of today.
        tree.run(&[ConfigOp::Disable("RUST".into())]);
    }

    tree.run(&config_fixes(signing_key));

    emit(Level::Info, "Resolving the dependencies of the configuration");
    let mut cmd = Command::new("make");
    cmd.arg("olddefconfig").current_dir(dir);
    let code = run_stream(cmd, cancel, &mut |_| {})?;
    if code != 0 {
        return Err("make olddefconfig failed".to_string());
    }
    // olddefconfig puts the choice of debug information back to the default.
    tree.run(&debug_info_fixes());

    // From here on: what olddefconfig dropped silently. A kernel that is named
    // after something it does not have is worse than no kernel.
    if let Some(symbol) = march_symbol {
        if !tree.is_on(symbol) {
            return Err(format!("{symbol} was dropped by olddefconfig: the patch of the architecture level is missing or did not apply. A kernel with the wrong level in its name is not built"));
        }
    }
    if signing_key.is_none() && tree.is_on("MODULE_SIG") {
        return Err("MODULE_SIG is on again after olddefconfig: something selects it. A kernel that taints itself with every unsigned module is not built".to_string());
    }
    for patch in &recipe.patches {
        if let Some(symbol) = scheduler_symbol(patch) {
            if !tree.is_on(symbol) {
                return Err(format!("{symbol} is not on: the {patch} patch is missing or did not apply. A kernel with the name of a patch it does not carry is not built"));
            }
        }
    }
    if recipe.has("rt") && !matches!(tree.state("DRM_I915").as_str(), "y" | "m") {
        return Err("DRM_I915 was dropped by olddefconfig: the rt-i915 patch is missing or did not apply. A real-time kernel without the Intel graphics driver gives a black screen on any Intel graphics".to_string());
    }
    if !tree.is_on("CGROUP_DMEM") {
        emit(Level::Warn, "CGROUP_DMEM was dropped by olddefconfig: this kernel will not have it");
    }
    if rust_binder {
        if !tree.is_on("ANDROID_BINDER_IPC_RUST") {
            let mut reason = "a dependency is missing".to_string();
            if !tree.is_on("RUST") {
                // RUST_IS_AVAILABLE is off when the toolchain is missing or too old:
                // rustavailable says which part fails.
                let (_, text) = run_capture({
                    let mut c = Command::new("make");
                    c.arg("rustavailable").current_dir(dir);
                    c
                });
                let text = text.trim();
                reason = if text.is_empty() { "Rust is not available".to_string() } else { format!("Rust is not available: {text}") };
            }
            return Err(format!("ANDROID_BINDER_IPC_RUST was dropped by olddefconfig ({reason}). A kernel without Binder is not built"));
        }
        let config = fs::read_to_string(dir.join(".config")).unwrap_or_default();
        let devices = config.lines().find(|l| l.starts_with("CONFIG_ANDROID_BINDER_DEVICES=")).unwrap_or("");
        if !devices.contains("hwbinder") || !devices.contains("vndbinder") {
            return Err("ANDROID_BINDER_DEVICES does not list hwbinder and vndbinder: Waydroid would not start".to_string());
        }
    }
    if recipe.sched_ext && !tree.is_on("SCHED_CLASS_EXT") {
        emit(Level::Warn, "SCHED_CLASS_EXT was dropped by olddefconfig: this kernel will not have sched_ext");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_architecture_level() {
        assert_eq!(march_ops("v1"), (None, vec![]));
        let (symbol, ops) = march_ops("v3");
        assert_eq!(symbol, Some("X86_64_ISA_V3"));
        assert!(ops.contains(&ConfigOp::Enable("X86_64_ISA_V3".into())));
        assert!(ops.contains(&ConfigOp::Disable("X86_64_ISA_V4".into())));
        assert!(!ops.contains(&ConfigOp::Disable("X86_64_ISA_V3".into())));
    }

    #[test]
    fn signing_changes_the_module_fixes() {
        let signed = config_fixes(Some(Path::new("/home/u/.config/k/MOK_combined.pem")));
        assert!(signed.contains(&ConfigOp::Enable("MODULE_SIG".into())));
        assert!(signed.iter().any(|o| matches!(o, ConfigOp::SetStr(n, v) if n == "MODULE_SIG_KEY" && v.ends_with("MOK_combined.pem"))));
        assert!(!signed.contains(&ConfigOp::Disable("MODULE_SIG".into())));
        let plain = config_fixes(None);
        assert!(plain.contains(&ConfigOp::Disable("MODULE_SIG".into())));
        assert!(plain.contains(&ConfigOp::Disable("SECURITY_LOCKDOWN_LSM".into())));
        assert!(plain.contains(&ConfigOp::Disable("MODVERSIONS".into())));
    }

    #[test]
    fn the_symbols_the_patches_add() {
        assert_eq!(scheduler_symbol("bore"), Some("SCHED_BORE"));
        assert_eq!(scheduler_symbol("x3d"), Some("SCHED_BORE"));
        assert_eq!(scheduler_symbol("zen"), Some("SCHED_ALT"));
        assert_eq!(scheduler_symbol("rt"), None);
    }

    #[test]
    fn ntsync_arrived_in_6_14() {
        assert!(ntsync_in_kernel("6.14"));
        assert!(ntsync_in_kernel("7.3.2"));
        assert!(!ntsync_in_kernel("6.12.80"));
    }

    #[test]
    fn the_rust_binder_is_read_from_the_tree() {
        let dir = std::env::temp_dir().join(format!("ki-kconfig-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("drivers/android")).unwrap();
        fs::write(dir.join("drivers/android/Kconfig"), "config ANDROID_BINDER_IPC\n\tbool\nconfig ANDROID_BINDER_IPC_RUST\n\tbool\n").unwrap();
        assert!(has_rust_binder(&dir));
        fs::write(dir.join("drivers/android/Kconfig"), "config ANDROID_BINDER_IPC\n\tbool\n").unwrap();
        assert!(!has_rust_binder(&dir));
        let _ = fs::remove_dir_all(&dir);
    }
}
