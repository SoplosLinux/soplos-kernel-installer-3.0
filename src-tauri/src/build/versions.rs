// The kernels that can be built: the list of kernel.org, which publishes it as
// JSON. The 2.x program read the HTML of the front page instead.

use serde::Serialize;
use serde_json::Value;
use std::process::Command;

#[derive(Serialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct KernelVersion {
    pub version: String,
    /// "stable", "longterm" or "rc".
    pub channel: String,
    /// The line of kernels no longer gets fixes.
    pub eol: bool,
    /// "YYYY-MM-DD".
    pub released: String,
    pub url: String,
    /// The newest stable one: the one the assistant offers first.
    pub recommended: bool,
}

pub const RELEASES_URL: &str = "https://www.kernel.org/releases.json";

pub fn parse_releases(json: &str) -> Vec<KernelVersion> {
    let Ok(value) = serde_json::from_str::<Value>(json) else { return Vec::new() };
    let latest = value.pointer("/latest_stable/version").and_then(Value::as_str).unwrap_or("").to_string();
    let mut out: Vec<KernelVersion> = Vec::new();
    for release in value.get("releases").and_then(Value::as_array).into_iter().flatten() {
        let text = |k: &str| release.get(k).and_then(Value::as_str).unwrap_or("").to_string();
        let (moniker, version, url) = (text("moniker"), text("version"), text("source"));
        if version.is_empty() || url.is_empty() {
            continue;
        }
        let channel = match moniker.as_str() {
            "stable" => "stable",
            "longterm" => "longterm",
            // The mainline kernel is only listed while it is a release candidate.
            "mainline" if version.contains("-rc") => "rc",
            _ => continue,
        };
        if out.iter().any(|k| k.version == version) {
            continue;
        }
        out.push(KernelVersion {
            recommended: channel == "stable" && version == latest,
            eol: release.get("iseol").and_then(Value::as_bool).unwrap_or(false),
            released: release.pointer("/released/isodate").and_then(Value::as_str).unwrap_or("").to_string(),
            channel: channel.to_string(),
            version,
            url,
        });
    }
    out.sort_by(|a, b| crate::privileged::version_cmp(&b.version, &a.version));
    out
}

pub fn fetch() -> Result<Vec<KernelVersion>, String> {
    let out = Command::new("curl")
        .args(["-fsSL", "--max-time", "30", "-A", "SoplosKernelInstaller/3.0", RELEASES_URL])
        .output()
        .map_err(|e| format!("curl: {e}"))?;
    if !out.status.success() {
        return Err("The list of kernels could not be downloaded".to_string());
    }
    let versions = parse_releases(&String::from_utf8_lossy(&out.stdout));
    if versions.is_empty() {
        Err("The list of kernels is empty".to_string())
    } else {
        Ok(versions)
    }
}

/// (major, minor) of a version like "7.3.2" or "7.4-rc2".
pub fn major_minor(version: &str) -> Option<(u32, u32)> {
    let mut parts = version.split(|c: char| c == '.' || c == '-');
    let major = parts.next()?.parse().ok()?;
    let minor = parts.next()?.parse().ok()?;
    Some((major, minor))
}

/// "7.3" of "7.3.2".
pub fn line_of(version: &str) -> Option<String> {
    major_minor(version).map(|(a, b)| format!("{a}.{b}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"{"latest_stable":{"version":"7.3.2"},"releases":[
        {"moniker":"mainline","version":"7.4-rc2","iseol":false,"released":{"isodate":"2026-11-02"},"source":"https://git.kernel.org/torvalds/t/linux-7.4-rc2.tar.gz"},
        {"moniker":"stable","version":"7.3.2","iseol":false,"released":{"isodate":"2026-10-30"},"source":"https://cdn.kernel.org/pub/linux/kernel/v7.x/linux-7.3.2.tar.xz"},
        {"moniker":"stable","version":"7.2.12","iseol":true,"released":{"isodate":"2026-10-20"},"source":"https://cdn.kernel.org/pub/linux/kernel/v7.x/linux-7.2.12.tar.xz"},
        {"moniker":"longterm","version":"6.18.31","iseol":false,"released":{"isodate":"2026-10-25"},"source":"https://cdn.kernel.org/pub/linux/kernel/v6.x/linux-6.18.31.tar.xz"},
        {"moniker":"longterm","version":"6.12.80","iseol":false,"released":{"isodate":"2026-10-25"},"source":"https://cdn.kernel.org/pub/linux/kernel/v6.x/linux-6.12.80.tar.xz"},
        {"moniker":"linux-next","version":"next-20261105","iseol":false,"released":{"isodate":"2026-11-05"},"source":"https://x"}]}"#;

    #[test]
    fn the_list_is_read_and_ordered() {
        let v = parse_releases(SAMPLE);
        let names: Vec<&str> = v.iter().map(|k| k.version.as_str()).collect();
        assert_eq!(names, vec!["7.4-rc2", "7.3.2", "7.2.12", "6.18.31", "6.12.80"]);
        assert_eq!(v[0].channel, "rc");
        assert!(v[1].recommended);
        assert!(!v[0].recommended && !v[2].recommended);
        assert!(v[2].eol);
        assert_eq!(v[3].channel, "longterm");
        assert_eq!(v[1].released, "2026-10-30");
    }

    #[test]
    fn nonsense_gives_nothing() {
        assert!(parse_releases("not json").is_empty());
        assert!(parse_releases("{}").is_empty());
    }

    // Needs the network: cargo test -- --ignored
    #[test]
    #[ignore]
    fn the_real_list_of_kernel_org() {
        let v = fetch().expect("kernel.org");
        assert!(v.iter().any(|k| k.recommended), "one is recommended");
        assert!(v.iter().any(|k| k.channel == "longterm"));
        assert!(v.iter().all(|k| crate::build::recipe::valid_version(&k.version)), "every version is one the recipe accepts: {:?}", v.iter().map(|k| &k.version).collect::<Vec<_>>());
    }

    #[test]
    fn lines() {
        assert_eq!(major_minor("7.3.2"), Some((7, 3)));
        assert_eq!(major_minor("7.4-rc2"), Some((7, 4)));
        assert_eq!(line_of("6.12.80").as_deref(), Some("6.12"));
        assert_eq!(major_minor("x"), None);
    }
}
