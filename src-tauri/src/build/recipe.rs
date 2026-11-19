// What the user chose in the assistant: the recipe of the kernel, and what each
// goal means for the configuration. The profiles are the ones of the 2.x
// program (core/profiles.py), chosen now by answering what the computer is for.

use serde::{Deserialize, Serialize};
use std::process::Command;

pub const GOALS: [&str; 5] = ["gaming", "audio", "minimal", "optimized", "stock"];
pub const PATCHES: [&str; 5] = ["bore", "rt", "zen", "ntsync", "x3d"];
pub const MARCHES: [&str; 4] = ["v1", "v2", "v3", "v4"];

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Recipe {
    pub version: String,
    /// One of GOALS.
    pub goal: String,
    pub patches: Vec<String>,
    /// One of MARCHES.
    pub march: String,
    /// The name that goes in the release: "soplos" gives 7.3.2-soplos-gaming.
    pub name: String,
    /// Sign the kernel with the key of the user.
    pub sign: bool,
    pub cores: u32,
    /// Keep the sources of an earlier build of the same version.
    pub reuse_sources: bool,
    /// Turn on sched_ext, the scheduler framework.
    pub sched_ext: bool,
}

pub fn valid_name(name: &str) -> bool {
    !name.is_empty() && name.len() <= 40 && name.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
}

pub fn valid_version(version: &str) -> bool {
    let (base, rc) = match version.split_once("-rc") {
        Some((b, r)) => (b, Some(r)),
        None => (version, None),
    };
    if let Some(r) = rc {
        if r.is_empty() || !r.chars().all(|c| c.is_ascii_digit()) {
            return false;
        }
    }
    let parts: Vec<&str> = base.split('.').collect();
    (2..=3).contains(&parts.len()) && parts.iter().all(|p| !p.is_empty() && p.chars().all(|c| c.is_ascii_digit()))
}

impl Recipe {
    pub fn validate(&self) -> Result<(), String> {
        if !valid_version(&self.version) {
            return Err(format!("Not a kernel version: {}", self.version));
        }
        if !GOALS.contains(&self.goal.as_str()) {
            return Err(format!("Unknown goal: {}", self.goal));
        }
        if let Some(p) = self.patches.iter().find(|p| !PATCHES.contains(&p.as_str())) {
            return Err(format!("Unknown patch: {p}"));
        }
        if !MARCHES.contains(&self.march.as_str()) {
            return Err(format!("Unknown architecture level: {}", self.march));
        }
        if !valid_name(&self.name) {
            return Err("The name can only have letters, numbers, dots, hyphens and underscores".to_string());
        }
        if self.cores == 0 || self.cores > 512 {
            return Err("The number of cores is not valid".to_string());
        }
        Ok(())
    }

    pub fn has(&self, patch: &str) -> bool {
        self.patches.iter().any(|p| p == patch)
    }

    /// Whether this kernel gets sched_ext. Zen replaces the scheduler of the
    /// kernel and its patch makes sched_ext depend on not having it.
    pub fn wants_sched_ext(&self) -> bool {
        self.sched_ext && !self.has("zen")
    }

    pub fn is_stock(&self) -> bool {
        self.goal == "stock"
    }

    /// The part of the release after the version: "-soplos-v3-gaming".
    pub fn local_version(&self) -> String {
        let mut name = self.name.clone();
        // The level goes in the name, or a V3 kernel would look like a V1 one.
        if !self.is_stock() && self.march != "v1" {
            name = format!("{name}-{}", self.march);
        }
        match profile(&self.goal).suffix {
            "" => format!("-{name}"),
            suffix => format!("-{name}-{suffix}"),
        }
    }
}

// ── Profiles ─────────────────────────────────────────────────────────────────

pub struct Profile {
    pub suffix: &'static str,
    pub options: &'static [(&'static str, &'static str)],
    pub disable: &'static [&'static str],
}

const HZ_1000: [(&str, &str); 5] = [("CONFIG_HZ_1000", "y"), ("CONFIG_HZ_300", "n"), ("CONFIG_HZ_250", "n"), ("CONFIG_HZ_100", "n"), ("CONFIG_HZ", "1000")];

pub fn profile(goal: &str) -> Profile {
    match goal {
        "gaming" => Profile {
            suffix: "gaming",
            options: &[
                ("CONFIG_HZ_1000", "y"),
                ("CONFIG_HZ_300", "n"),
                ("CONFIG_HZ_250", "n"),
                ("CONFIG_HZ_100", "n"),
                ("CONFIG_HZ", "1000"),
                ("CONFIG_CPU_FREQ_GOV_PERFORMANCE", "y"),
                ("CONFIG_CPU_FREQ_DEFAULT_GOV_PERFORMANCE", "y"),
                ("CONFIG_TRANSPARENT_HUGEPAGE", "y"),
                ("CONFIG_TRANSPARENT_HUGEPAGE_ALWAYS", "y"),
                ("CONFIG_DEBUG_INFO", "n"),
                ("CONFIG_DEBUG_KERNEL", "n"),
                ("CONFIG_FTRACE", "n"),
                ("CONFIG_FUTEX", "y"),
                ("CONFIG_FUTEX_PI", "y"),
            ],
            disable: &[],
        },
        "audio" => Profile {
            suffix: "lowlatency",
            options: &[
                ("CONFIG_HZ_1000", "y"),
                ("CONFIG_HZ_300", "n"),
                ("CONFIG_HZ_250", "n"),
                ("CONFIG_HZ_100", "n"),
                ("CONFIG_HZ", "1000"),
                ("CONFIG_CPU_FREQ_GOV_PERFORMANCE", "y"),
                ("CONFIG_CPU_FREQ_DEFAULT_GOV_PERFORMANCE", "y"),
                ("CONFIG_TRANSPARENT_HUGEPAGE", "y"),
                ("CONFIG_TRANSPARENT_HUGEPAGE_ALWAYS", "n"),
                ("CONFIG_TRANSPARENT_HUGEPAGE_MADVISE", "y"),
                ("CONFIG_FUTEX", "y"),
                ("CONFIG_FUTEX_PI", "y"),
                ("CONFIG_DEBUG_INFO", "n"),
                ("CONFIG_DEBUG_KERNEL", "n"),
                ("CONFIG_FTRACE", "n"),
            ],
            disable: &[],
        },
        "minimal" => Profile {
            suffix: "minimal",
            options: &[
                ("CONFIG_HZ_250", "y"),
                ("CONFIG_HZ", "250"),
                ("CONFIG_CPU_FREQ_GOV_POWERSAVE", "y"),
                ("CONFIG_CPU_IDLE", "y"),
                ("CONFIG_DEBUG_INFO", "n"),
                ("CONFIG_DEBUG_KERNEL", "n"),
                ("CONFIG_FTRACE", "n"),
                ("CONFIG_NVME_CORE", "y"),
                ("CONFIG_BLK_DEV_NVME", "y"),
                ("CONFIG_SATA_AHCI", "y"),
            ],
            disable: &["JOYSTICK", "GAMEPORT", "REISERFS_FS", "HAMRADIO"],
        },
        // The options of the automatic profile come from the hardware.
        "optimized" => Profile { suffix: "optimized", options: &[], disable: &[] },
        _ => Profile { suffix: "", options: &[("CONFIG_DEBUG_INFO", "n"), ("CONFIG_DEBUG_KERNEL", "n")], disable: &[] },
    }
}

/// The patches the assistant suggests for a goal.
pub fn suggested_patches(goal: &str) -> Vec<&'static str> {
    match goal {
        "gaming" => vec!["bore", "ntsync"],
        "audio" => vec!["rt"],
        "optimized" => vec!["bore"],
        _ => vec![],
    }
}

// ── Configuration commands ───────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConfigOp {
    Enable(String),
    Disable(String),
    Module(String),
    SetVal(String, String),
    SetStr(String, String),
}

impl ConfigOp {
    /// The arguments for `scripts/config`.
    pub fn args(&self) -> Vec<String> {
        match self {
            ConfigOp::Enable(n) => vec!["--enable".into(), n.clone()],
            ConfigOp::Disable(n) => vec!["--disable".into(), n.clone()],
            ConfigOp::Module(n) => vec!["--module".into(), n.clone()],
            ConfigOp::SetVal(n, v) => vec!["--set-val".into(), n.clone(), v.clone()],
            ConfigOp::SetStr(n, v) => vec!["--set-str".into(), n.clone(), v.clone()],
        }
    }
}

fn op_for(key: &str, value: &str) -> ConfigOp {
    let name = key.strip_prefix("CONFIG_").unwrap_or(key).to_string();
    match value {
        "y" => ConfigOp::Enable(name),
        "n" => ConfigOp::Disable(name),
        "m" => ConfigOp::Module(name),
        other => ConfigOp::SetVal(name, other.trim_matches(['"', '\'']).to_string()),
    }
}

/// The configuration a profile asks for. `hardware` is what the automatic
/// profile found on this computer.
pub fn profile_ops(goal: &str, hardware: &[(String, String)]) -> Vec<ConfigOp> {
    let p = profile(goal);
    let mut ops: Vec<ConfigOp> = p.options.iter().map(|(k, v)| op_for(k, v)).collect();
    if goal == "optimized" {
        ops.extend(HZ_1000.iter().map(|(k, v)| op_for(k, v)));
        ops.extend(hardware.iter().map(|(k, v)| op_for(k, v)));
    }
    ops.extend(p.disable.iter().map(|m| ConfigOp::Disable(m.to_string())));
    ops
}

/// The drivers the automatic profile builds in for this computer, read from
/// `lspci`.
pub fn hardware_options(lspci: &str) -> Vec<(String, String)> {
    let lower = lspci.to_lowercase();
    let all: Vec<&str> = lower.lines().collect();
    let mut out: Vec<(String, String)> = Vec::new();
    let mut add = |k: &str| {
        if !out.iter().any(|(key, _)| key == k) {
            out.push((k.to_string(), "y".to_string()));
        }
    };
    for line in all.iter().filter(|l| ["vga", "display", "3d controller", "3d"].iter().any(|k| l.contains(k))) {
        if line.contains("nvidia") {
            add("CONFIG_FB_NVIDIA");
        }
        // "ati" alone is in "compatible": every VGA line would match it.
        if line.contains("amd") || line.contains("advanced micro") || line.contains("ati technologies") || line.contains("[ati]") {
            add("CONFIG_DRM_AMDGPU");
        }
        if line.contains("intel") {
            add("CONFIG_DRM_I915");
        }
        if line.contains("vmware") || line.contains("vmsvga") {
            add("CONFIG_DRM_VMWGFX");
        }
        if line.contains("virtualbox") || line.contains("vbox") {
            add("CONFIG_DRM_VBOXVIDEO");
        }
        if line.contains("qxl") {
            add("CONFIG_DRM_QXL");
        }
        if line.contains("virtio") {
            add("CONFIG_DRM_VIRTIO_GPU");
        }
    }
    if all.iter().any(|l| l.contains("virtio") || l.contains("qemu") || l.contains("red hat")) {
        for k in ["CONFIG_VIRTIO", "CONFIG_VIRTIO_PCI", "CONFIG_VIRTIO_BLK", "CONFIG_VIRTIO_NET"] {
            add(k);
        }
    }
    if all.iter().any(|l| l.contains("vmware")) {
        add("CONFIG_VMXNET3");
        add("CONFIG_VMCI");
    }
    if all.iter().any(|l| l.contains("virtualbox") || l.contains("vbox")) {
        add("CONFIG_VBOXGUEST");
    }
    out
}

pub fn detect_hardware() -> Vec<(String, String)> {
    let lspci = Command::new("lspci").output().map(|o| String::from_utf8_lossy(&o.stdout).to_string()).unwrap_or_default();
    hardware_options(&lspci)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn recipe() -> Recipe {
        Recipe {
            version: "7.3.2".into(),
            goal: "gaming".into(),
            patches: vec!["bore".into(), "ntsync".into()],
            march: "v3".into(),
            name: "soplos".into(),
            sign: false,
            cores: 8,
            reuse_sources: false,
            sched_ext: false,
        }
    }

    #[test]
    fn a_good_recipe_validates() {
        assert!(recipe().validate().is_ok());
    }

    #[test]
    fn bad_recipes_are_refused() {
        let mut r = recipe();
        r.version = "7.3; rm".into();
        assert!(r.validate().is_err());
        let mut r = recipe();
        r.goal = "other".into();
        assert!(r.validate().is_err());
        let mut r = recipe();
        r.patches = vec!["evil".into()];
        assert!(r.validate().is_err());
        let mut r = recipe();
        r.name = "a b".into();
        assert!(r.validate().is_err());
        let mut r = recipe();
        r.march = "v9".into();
        assert!(r.validate().is_err());
        let mut r = recipe();
        r.cores = 0;
        assert!(r.validate().is_err());
    }

    #[test]
    fn versions() {
        assert!(valid_version("7.3"));
        assert!(valid_version("7.3.2"));
        assert!(valid_version("7.4-rc2"));
        assert!(!valid_version("7"));
        assert!(!valid_version("7.3.2.1"));
        assert!(!valid_version("7.4-rc"));
        assert!(!valid_version("../7.3"));
    }

    #[test]
    fn the_release_name() {
        let mut r = recipe();
        assert_eq!(r.local_version(), "-soplos-v3-gaming");
        r.march = "v1".into();
        assert_eq!(r.local_version(), "-soplos-gaming");
        r.goal = "stock".into();
        r.march = "v4".into();
        assert_eq!(r.local_version(), "-soplos");
        r.goal = "audio".into();
        assert_eq!(r.local_version(), "-soplos-v4-lowlatency");
    }

    #[test]
    fn profile_commands() {
        let ops = profile_ops("minimal", &[]);
        assert!(ops.contains(&ConfigOp::Enable("HZ_250".into())));
        assert!(ops.contains(&ConfigOp::SetVal("HZ".into(), "250".into())));
        assert!(ops.contains(&ConfigOp::Disable("JOYSTICK".into())));
        assert_eq!(ConfigOp::SetVal("HZ".into(), "250".into()).args(), vec!["--set-val", "HZ", "250"]);
        let auto = profile_ops("optimized", &[("CONFIG_DRM_AMDGPU".into(), "y".into())]);
        assert!(auto.contains(&ConfigOp::Enable("HZ_1000".into())));
        assert!(auto.contains(&ConfigOp::Enable("DRM_AMDGPU".into())));
    }

    #[test]
    fn hardware_is_read_from_lspci() {
        let lspci = "00:02.0 VGA compatible controller: Intel Corporation UHD Graphics\n01:00.0 3D controller: NVIDIA Corporation GA107\n00:03.0 Ethernet controller: Red Hat, Inc. Virtio network device\n";
        let h = hardware_options(lspci);
        let keys: Vec<&str> = h.iter().map(|(k, _)| k.as_str()).collect();
        assert!(keys.contains(&"CONFIG_DRM_I915"));
        assert!(keys.contains(&"CONFIG_FB_NVIDIA"));
        assert!(keys.contains(&"CONFIG_VIRTIO_NET"));
        assert!(!keys.contains(&"CONFIG_DRM_AMDGPU"));
    }

    #[test]
    fn suggestions() {
        assert_eq!(suggested_patches("gaming"), vec!["bore", "ntsync"]);
        assert!(suggested_patches("stock").is_empty());
    }
}
