// The patches: where each one comes from, and putting them on the sources.
// Ported from `core/downloader.py` and `core/patcher.py` of the 2.x program.
// The Soplos rebases are tried first, as the user asked; a patch that downloads
// but does not apply to this point release is not accepted.

use super::config::Level;
use super::runner::run_capture;
use super::versions::major_minor;
use serde_json::Value;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};

const RAW_SOPLOS: &str = "https://raw.githubusercontent.com/SoplosLinux";
const RT_INDEX: &str = "https://www.kernel.org/pub/linux/kernel/projects/rt";

/// The patch files that make one patch of the recipe.
pub type PatchSet = (String, Vec<PathBuf>);

pub fn bore_soplos_url(mm: &str) -> String {
    format!("{RAW_SOPLOS}/bore-soplos/main/patches/0001-bore-{mm}.patch")
}

pub fn bore_cachy_url(mm: &str) -> String {
    format!("https://raw.githubusercontent.com/CachyOS/kernel-patches/master/{mm}/sched/0001-bore.patch")
}

pub fn bore_upstream_api(tree: &str, mm: &str) -> String {
    format!("https://api.github.com/repos/firelzrd/bore-scheduler/contents/patches/{tree}/linux-{mm}-bore")
}

/// The folder of a tree of the repository of firelzrd, for the layout in which
/// the files are loose and the name carries the point release the patch was made
/// for: `0001-linux6.18.48-bore-7.0.0.patch`.
pub fn bore_tree_api(tree: &str) -> String {
    format!("https://api.github.com/repos/firelzrd/bore-scheduler/contents/patches/{tree}")
}

/// The files of a tree, in the loose layout, that are for a kernel line, the one
/// made for the newest point release first (and, for the same one, the newest
/// version of BORE).
pub fn parse_bore_loose(json: &str, mm: &str) -> Vec<String> {
    let prefix = format!("0001-linux{mm}.");
    let mut found: Vec<(u32, Vec<u32>, String)> = parse_github_files(json)
        .into_iter()
        .filter_map(|name| {
            let rest = name.strip_prefix(&prefix)?;
            let (point, after) = rest.split_once("-bore-")?;
            let point = point.parse::<u32>().ok()?;
            let bore = after.strip_suffix(".patch")?.split('.').map(|n| n.parse::<u32>().ok()).collect::<Option<Vec<u32>>>()?;
            Some((point, bore, name))
        })
        .collect();
    found.sort_by(|a, b| (b.0, &b.1).cmp(&(a.0, &a.1)));
    found.into_iter().map(|(_, _, name)| name).collect()
}

pub fn zen_soplos_urls(version: &str, mm: &str) -> [String; 2] {
    [
        format!("{RAW_SOPLOS}/zen-soplos/main/patches/0001-zen-{version}.patch"),
        format!("{RAW_SOPLOS}/zen-soplos/main/patches/0001-zen-{mm}.patch"),
    ]
}

pub fn rt_i915_url(mm: &str) -> String {
    format!("{RAW_SOPLOS}/rt-i915-soplos/main/patches/0001-rt-i915-{mm}.patch")
}

/// The patch of the X3D V-Cache scheduler is kept per line for the ones that
/// have their own file; the newer ones share the `7.x` file.
pub fn x3d_tag(version: &str) -> String {
    let (major, minor) = major_minor(version).unwrap_or((0, 0));
    let mm = format!("{major}.{minor}");
    if ["6.12", "7.0", "7.1", "7.2"].contains(&mm.as_str()) {
        mm
    } else {
        format!("{major}.x")
    }
}

pub fn x3d_url(version: &str) -> String {
    format!("{RAW_SOPLOS}/x3d-soplos/main/patches/0001-sched-amd-x3d-vcache-{}.patch", x3d_tag(version))
}

/// The ISA level patch is one file for the whole major version: asking for the
/// exact line gives a 404, and the build would silently be a V1.
pub fn march_url(version: &str) -> String {
    let (major, _) = major_minor(version).unwrap_or((0, 0));
    format!("{RAW_SOPLOS}/soplos-cpu-kernel-patch/main/patches/0001-x86-64-isa-level-{major}.x.patch")
}

/// PREEMPT_RT is part of the kernel since 6.12.
pub fn rt_is_builtin(version: &str) -> bool {
    major_minor(version).map(|m| m >= (6, 12)).unwrap_or(false)
}

/// The newest RT patch of a line, read from the index of kernel.org:
/// `patch-6.9-rt5.patch.xz`, `patch-7.0-rc1-rt1.patch.xz`.
pub fn parse_rt_index(html: &str, mm: &str) -> Option<String> {
    let prefix = format!("patch-{mm}");
    let mut best: Option<(u32, String)> = None;
    let mut rest = html;
    while let Some(i) = rest.find(&prefix) {
        let tail = &rest[i..];
        let end = tail.find(|c: char| c == '"' || c == '<' || c == ' ' || c == '\'').unwrap_or(tail.len());
        let name = &tail[..end];
        if name.ends_with(".patch.xz") {
            if let Some(n) = name.rsplit("-rt").next().and_then(|s| s.strip_suffix(".patch.xz")).and_then(|s| s.parse::<u32>().ok()) {
                if name.contains("-rt") && best.as_ref().map(|(b, _)| n > *b).unwrap_or(true) {
                    best = Some((n, name.to_string()));
                }
            }
        }
        rest = &rest[i + prefix.len()..];
    }
    best.map(|(_, name)| name)
}

/// The names of the patch files in a folder of the GitHub API.
pub fn parse_github_files(json: &str) -> Vec<String> {
    let Ok(Value::Array(items)) = serde_json::from_str::<Value>(json) else { return Vec::new() };
    let mut names: Vec<String> = items.iter().filter_map(|i| i.get("name").and_then(Value::as_str)).filter(|n| n.ends_with(".patch")).map(str::to_string).collect();
    names.sort();
    names
}

/// The patch of the releases of Zen that matches a version.
pub fn zen_asset(releases_json: &str, version: &str) -> Option<(String, String)> {
    let Ok(Value::Array(releases)) = serde_json::from_str::<Value>(releases_json) else { return None };
    let prefix = format!("v{version}-zen");
    for release in releases {
        if !release.get("tag_name").and_then(Value::as_str).unwrap_or("").starts_with(&prefix) {
            continue;
        }
        for asset in release.get("assets").and_then(Value::as_array).into_iter().flatten() {
            let name = asset.get("name").and_then(Value::as_str).unwrap_or("");
            if name.ends_with(".patch.zst") {
                if let Some(url) = asset.get("browser_download_url").and_then(Value::as_str) {
                    return Some((name.to_string(), url.to_string()));
                }
            }
        }
    }
    None
}

// ── Downloading ──────────────────────────────────────────────────────────────

pub fn fetch_file(url: &str, dest: &Path) -> bool {
    let (code, _) = run_capture({
        let mut c = Command::new("curl");
        c.args(["-fsSL", "--retry", "2", "--connect-timeout", "15", "--max-time", "900", "-A", "SoplosKernelInstaller/3.0", "-o"]).arg(dest).arg(url);
        c
    });
    code == 0 && fs::metadata(dest).map(|m| m.len() > 0).unwrap_or(false)
}

fn fetch_text(url: &str) -> Option<String> {
    let (code, out) = run_capture({
        let mut c = Command::new("curl");
        c.args(["-fsSL", "--connect-timeout", "15", "--max-time", "60", "-A", "SoplosKernelInstaller/3.0", "-H", "Accept: application/vnd.github+json", url]);
        c
    });
    (code == 0).then_some(out)
}

/// A patch that downloads is not a patch that applies: this is what tells a
/// stale one, made for an older point release.
pub fn applies(source: &Path, patch: &Path) -> bool {
    run_capture({
        let mut c = Command::new("patch");
        c.args(["-p1", "--batch", "--forward", "--dry-run", "-i"]).arg(patch).current_dir(source);
        c
    })
    .0 == 0
}

fn all_apply(source: &Path, files: &[PathBuf]) -> bool {
    files.iter().all(|f| applies(source, f))
}

fn download_bore(version: &str, mm: &str, dir: &Path, source: &Path, emit: &mut dyn FnMut(Level, &str)) -> Option<Vec<PathBuf>> {
    // 1. The rebase of Soplos.
    let dest = dir.join(format!("bore-soplos-{mm}.patch"));
    if fetch_file(&bore_soplos_url(mm), &dest) {
        if all_apply(source, &[dest.clone()]) {
            return Some(vec![dest]);
        }
        emit(Level::Warn, &format!("The BORE patch of Soplos does not apply to {version}: trying the upstream one"));
    }
    // 2. The upstream repository, stable and then legacy.
    for tree in ["stable", "legacy"] {
        let Some(json) = fetch_text(&bore_upstream_api(tree, mm)) else { continue };
        let names = parse_github_files(&json);
        if names.is_empty() {
            continue;
        }
        let mut files = Vec::new();
        for name in &names {
            let url = format!("https://raw.githubusercontent.com/firelzrd/bore-scheduler/main/patches/{tree}/linux-{mm}-bore/{name}");
            let dest = dir.join(format!("bore-{name}"));
            if !fetch_file(&url, &dest) {
                files.clear();
                break;
            }
            files.push(dest);
        }
        if !files.is_empty() {
            if all_apply(source, &files) {
                return Some(files);
            }
            emit(Level::Warn, &format!("The BORE patch of upstream ({tree}) does not apply to {version}: trying the next one"));
        }
    }
    // 3. The upstream repository again, in the layout it has now: loose files with
    //    the point release in the name. The newest that applies is the one.
    if let Some(json) = fetch_text(&bore_tree_api("stable")) {
        for name in parse_bore_loose(&json, mm) {
            let url = format!("https://raw.githubusercontent.com/firelzrd/bore-scheduler/main/patches/stable/{name}");
            let dest = dir.join(format!("bore-{name}"));
            if fetch_file(&url, &dest) {
                if all_apply(source, &[dest.clone()]) {
                    return Some(vec![dest]);
                }
                emit(Level::Warn, &format!("The BORE patch of upstream ({name}) does not apply to {version}: trying the next one"));
            }
        }
    }
    // 4. CachyOS.
    let dest = dir.join(format!("bore-cachy-{mm}.patch"));
    if fetch_file(&bore_cachy_url(mm), &dest) {
        if all_apply(source, &[dest.clone()]) {
            return Some(vec![dest]);
        }
        emit(Level::Warn, &format!("The BORE patch of CachyOS does not apply to {version} either"));
    }
    None
}

fn download_zen(version: &str, mm: &str, dir: &Path, source: &Path, emit: &mut dyn FnMut(Level, &str)) -> Option<Vec<PathBuf>> {
    // Zen only patches stable releases.
    if version.contains("-rc") {
        emit(Level::Warn, "Zen has no patch for a release candidate");
        return None;
    }
    for (i, url) in zen_soplos_urls(version, mm).iter().enumerate() {
        let dest = dir.join(format!("zen-soplos-{i}.patch"));
        if fetch_file(url, &dest) {
            if all_apply(source, &[dest.clone()]) {
                return Some(vec![dest]);
            }
            emit(Level::Warn, "The Zen patch of Soplos does not apply: trying the upstream one");
            break;
        }
    }
    // The releases of zen-kernel, page by page.
    let mut page = 1;
    loop {
        let url = format!("https://api.github.com/repos/zen-kernel/zen-kernel/releases?per_page=100&page={page}");
        let Some(json) = fetch_text(&url) else {
            emit(Level::Warn, "GitHub could not be reached for the Zen patch");
            return None;
        };
        if let Some((name, asset)) = zen_asset(&json, version) {
            let zst = dir.join(&name);
            let dest = dir.join(format!("zen-{version}.patch"));
            if !fetch_file(&asset, &zst) {
                return None;
            }
            let (code, _) = run_capture({
                let mut c = Command::new("zstd");
                c.args(["-d", "-f", "-o"]).arg(&dest).arg(&zst);
                c
            });
            if code == 0 && all_apply(source, &[dest.clone()]) {
                return Some(vec![dest]);
            }
            return None;
        }
        let count = serde_json::from_str::<Value>(&json).ok().and_then(|v| v.as_array().map(Vec::len)).unwrap_or(0);
        if count < 100 {
            return None;
        }
        page += 1;
    }
}

fn download_rt(version: &str, mm: &str, dir: &Path) -> Option<Vec<PathBuf>> {
    let index_url = format!("{RT_INDEX}/{mm}/");
    let html = fetch_text(&index_url)?;
    let name = parse_rt_index(&html, mm)?;
    let xz = dir.join(&name);
    if !fetch_file(&format!("{index_url}{name}"), &xz) {
        return None;
    }
    let (code, _) = run_capture({
        let mut c = Command::new("xz");
        c.args(["-d", "-f"]).arg(&xz);
        c
    });
    let plain = xz.with_extension("");
    let dest = dir.join(format!("rt-{version}.patch"));
    (code == 0 && fs::rename(plain, &dest).is_ok()).then(|| vec![dest])
}

fn download_rt_i915(version: &str, mm: &str, dir: &Path, source: &Path, emit: &mut dyn FnMut(Level, &str)) -> Option<Vec<PathBuf>> {
    let dest = dir.join(format!("rt-i915-soplos-{mm}.patch"));
    if !fetch_file(&rt_i915_url(mm), &dest) {
        emit(Level::Warn, &format!("There is no rt-i915 patch of Soplos for {mm}"));
        return None;
    }
    if !applies(source, &dest) {
        emit(Level::Warn, &format!("The rt-i915 patch of Soplos does not apply to {version}"));
        return None;
    }
    Some(vec![dest])
}

/// Downloads the patches of a recipe. A patch that cannot be had stops the
/// build: going on without it ships a kernel named after a patch it does not
/// carry. NTSYNC is a configuration option, not a file.
pub fn download(
    version: &str,
    ids: &[String],
    dir: &Path,
    source: &Path,
    cancel: &AtomicBool,
    emit: &mut dyn FnMut(Level, &str),
) -> Result<Vec<PatchSet>, String> {
    fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    let mm = super::versions::line_of(version).ok_or("Not a kernel version")?;
    let mut out: Vec<PatchSet> = Vec::new();
    let mut missing: Vec<&str> = Vec::new();
    for id in ids {
        if cancel.load(Ordering::Relaxed) {
            return Err("cancelled".to_string());
        }
        if id == "ntsync" {
            continue;
        }
        emit(Level::Info, &format!("Downloading the {id} patch"));
        let files = match id.as_str() {
            "bore" => download_bore(version, &mm, dir, source, emit),
            "zen" => download_zen(version, &mm, dir, source, emit),
            "rt" if rt_is_builtin(version) => {
                emit(Level::Info, &format!("PREEMPT_RT is part of kernel {version}: no RT patch is needed"));
                // Mainline still marks i915 as depending on !PREEMPT_RT, so the
                // fix of Soplos goes in whatever the graphics of this computer.
                download_rt_i915(version, &mm, dir, source, emit)
            }
            "rt" => download_rt(version, &mm, dir),
            "x3d" => {
                let dest = dir.join(format!("soplos-x3d-vcache-{}.patch", x3d_tag(version)));
                fetch_file(&x3d_url(version), &dest).then(|| vec![dest])
            }
            "march" => {
                let dest = dir.join("soplos-x86-64-isa-level.patch");
                fetch_file(&march_url(version), &dest).then(|| vec![dest])
            }
            _ => None,
        };
        match files {
            Some(f) => out.push((id.clone(), f)),
            None => missing.push(id),
        }
    }
    if !missing.is_empty() {
        return Err(format!("The patch is not available for kernel {version}: {}. A kernel with the name of a patch it does not carry is not built", missing.join(", ")));
    }
    Ok(out)
}

// ── Applying ─────────────────────────────────────────────────────────────────

fn order(ids: &[String]) -> Vec<String> {
    let first = ["rt", "bore", "zen", "x3d"];
    let mut out: Vec<String> = first.iter().filter(|p| ids.iter().any(|i| i == *p)).map(|p| p.to_string()).collect();
    out.extend(ids.iter().filter(|i| !first.contains(&i.as_str())).cloned());
    out
}

enum Outcome {
    Applied,
    /// Every hunk of it is in the tree already.
    Already,
    Failed,
}

fn patch_run(source: &Path, patch: &Path, extra: &[&str]) -> i32 {
    run_capture({
        let mut c = Command::new("patch");
        c.args(["-p1", "--batch"]).args(extra).arg("-i").arg(patch).current_dir(source);
        c
    })
    .0
}

/// One patch, whole or not at all. It is tried with --dry-run first, and only a
/// patch that fits entirely is applied: GNU patch would otherwise apply the hunks
/// that fit and leave the rest in a .rej file, a file with some of its changes.
/// A patch whose hunks are all in the tree already is not a failure, but one that
/// has only some of them is.
fn apply_whole(source: &Path, patch: &Path) -> Outcome {
    if applies(source, patch) {
        return if patch_run(source, patch, &["--forward"]) == 0 { Outcome::Applied } else { Outcome::Failed };
    }
    // Taking it out would work only if all of it is there.
    if patch_run(source, patch, &["-R", "--dry-run"]) == 0 {
        Outcome::Already
    } else {
        Outcome::Failed
    }
}

/// Takes out, last first, the patches this call applied. False when one of them
/// would not come out.
fn roll_back(source: &Path, done: &[PathBuf]) -> bool {
    done.iter().rev().all(|file| patch_run(source, file, &["-R"]) == 0)
}

/// Applies the patches of the recipe: all of them or none. If one does not apply,
/// or the build is cancelled half way, the ones that were applied come out again
/// and the tree is as it was; if they cannot come out, the tree is deleted, so a
/// source with some of the patches is never built or reused.
pub fn apply(source: &Path, sets: &[PatchSet], cancel: &AtomicBool, emit: &mut dyn FnMut(Level, &str)) -> Result<(), String> {
    let ids: Vec<String> = sets.iter().map(|(id, _)| id.clone()).collect();
    let mut done: Vec<PathBuf> = Vec::new();
    let mut failure: Option<String> = None;
    'queue: for id in order(&ids) {
        if cancel.load(Ordering::Relaxed) {
            failure = Some("cancelled".to_string());
            break;
        }
        emit(Level::Info, &format!("Applying the {id} patch"));
        let Some((_, files)) = sets.iter().find(|(i, _)| *i == id) else { continue };
        for file in files {
            match apply_whole(source, file) {
                Outcome::Applied => done.push(file.clone()),
                Outcome::Already => {}
                Outcome::Failed => {
                    failure = Some(format!("The {id} patch did not apply ({})", file.display()));
                    break 'queue;
                }
            }
        }
    }
    if let Some(reason) = failure {
        if !done.is_empty() {
            emit(Level::Warn, "Taking out the patches that were applied: they are all applied or none is");
            if !roll_back(source, &done) {
                emit(Level::Warn, "They did not come out: the sources are deleted, so a half patched kernel is never built");
                let _ = fs::remove_dir_all(source);
            }
        }
        return Err(if reason == "cancelled" { reason } else { format!("{reason}. No patch was left applied") });
    }
    let _ = fs::write(source.join(".applied_patches"), ids.join("\n"));
    Ok(())
}

/// The patches recorded as applied on a source tree.
pub fn applied(source: &Path) -> Vec<String> {
    fs::read_to_string(source.join(".applied_patches")).map(|t| t.lines().map(str::trim).filter(|l| !l.is_empty()).map(str::to_string).collect()).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls() {
        assert_eq!(bore_soplos_url("7.3"), "https://raw.githubusercontent.com/SoplosLinux/bore-soplos/main/patches/0001-bore-7.3.patch");
        assert_eq!(rt_i915_url("7.3"), "https://raw.githubusercontent.com/SoplosLinux/rt-i915-soplos/main/patches/0001-rt-i915-7.3.patch");
        assert!(march_url("7.3.2").ends_with("0001-x86-64-isa-level-7.x.patch"));
        assert!(march_url("6.18.3").ends_with("0001-x86-64-isa-level-6.x.patch"));
        assert!(x3d_url("7.2.9").ends_with("vcache-7.2.patch"));
        assert!(x3d_url("7.3.1").ends_with("vcache-7.x.patch"));
        assert!(x3d_url("6.12.80").ends_with("vcache-6.12.patch"));
        assert!(x3d_url("6.18.3").ends_with("vcache-6.x.patch"));
        let [exact, line] = zen_soplos_urls("7.2.9", "7.2");
        assert!(exact.ends_with("0001-zen-7.2.9.patch") && line.ends_with("0001-zen-7.2.patch"));
    }

    #[test]
    fn rt_is_part_of_the_kernel_from_6_12() {
        assert!(rt_is_builtin("6.12.1"));
        assert!(rt_is_builtin("7.3"));
        assert!(!rt_is_builtin("6.11.9"));
    }

    #[test]
    fn the_newest_rt_patch_of_a_line() {
        let html = r#"<a href="patch-6.9-rt5.patch.xz">x</a> <a href="patch-6.9-rt12.patch.xz">y</a> <a href="patch-6.9-rt5.patch.gz">z</a> <a href="patch-6.9-rt9.patch.xz">w</a> <a href="patch-6.10-rt1.patch.xz">q</a>"#;
        assert_eq!(parse_rt_index(html, "6.9").as_deref(), Some("patch-6.9-rt12.patch.xz"));
        assert_eq!(parse_rt_index(html, "6.8"), None);
        assert_eq!(parse_rt_index(r#"<a href="patch-7.0-rc1-rt1.patch.xz">"#, "7.0").as_deref(), Some("patch-7.0-rc1-rt1.patch.xz"));
    }

    #[test]
    fn the_loose_bore_files_of_a_line_newest_first() {
        let json = r#"[{"name":"0001-linux6.12.37-bore-7.0.0.patch"},{"name":"0001-linux6.18.48-bore-7.0.0.patch"},{"name":"0001-linux6.18.9-bore-7.0.0.patch"},{"name":"0001-linux6.18.48-bore-7.0.1.patch"},{"name":"README.md"},{"name":"0001-linux6.180.1-bore-7.0.0.patch"}]"#;
        assert_eq!(
            parse_bore_loose(json, "6.18"),
            vec!["0001-linux6.18.48-bore-7.0.1.patch", "0001-linux6.18.48-bore-7.0.0.patch", "0001-linux6.18.9-bore-7.0.0.patch"]
        );
        assert_eq!(parse_bore_loose(json, "6.12"), vec!["0001-linux6.12.37-bore-7.0.0.patch"]);
        assert!(parse_bore_loose(json, "7.3").is_empty());
    }

    #[test]
    fn github_listings() {
        let json = r#"[{"name":"0001-bore.patch"},{"name":"README.md"},{"name":"0002-bore-fix.patch"}]"#;
        assert_eq!(parse_github_files(json), vec!["0001-bore.patch", "0002-bore-fix.patch"]);
        assert!(parse_github_files("{}").is_empty());
        let releases = r#"[{"tag_name":"v7.2.9-zen1","assets":[{"name":"x.patch.zst","browser_download_url":"https://h/x.patch.zst"}]},{"tag_name":"v7.2.8-zen1","assets":[]}]"#;
        assert_eq!(zen_asset(releases, "7.2.9"), Some(("x.patch.zst".into(), "https://h/x.patch.zst".into())));
        assert_eq!(zen_asset(releases, "7.2.8"), None);
        assert_eq!(zen_asset(releases, "7.3.0"), None);
    }

    #[test]
    fn the_order_of_application() {
        let ids: Vec<String> = ["march", "x3d", "bore", "rt"].iter().map(|s| s.to_string()).collect();
        assert_eq!(order(&ids), vec!["rt", "bore", "x3d", "march"]);
    }

    #[test]
    fn patches_apply_and_the_record_is_kept() {
        let dir = std::env::temp_dir().join(format!("ki-patch-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("a.txt"), "one\ntwo\n").unwrap();
        let patch = "--- a/a.txt\n+++ b/a.txt\n@@ -1,2 +1,3 @@\n one\n two\n+three\n";
        let p = dir.join("p.patch");
        fs::write(&p, patch).unwrap();
        assert!(applies(&dir, &p));
        let cancel = AtomicBool::new(false);
        let sets = vec![("bore".to_string(), vec![p.clone()])];
        apply(&dir, &sets, &cancel, &mut |_, _| {}).unwrap();
        assert_eq!(fs::read_to_string(dir.join("a.txt")).unwrap(), "one\ntwo\nthree\n");
        assert_eq!(applied(&dir), vec!["bore"]);
        // Applying it again is not a failure.
        apply(&dir, &sets, &cancel, &mut |_, _| {}).unwrap();
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_patches_are_all_applied_or_none_is() {
        let dir = std::env::temp_dir().join(format!("ki-patch-all-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("a.txt"), "one\ntwo\n").unwrap();
        fs::write(dir.join("b.txt"), "x\ny\n").unwrap();
        // The first fits, the second has one hunk that fits and one that does not.
        let good = dir.join("good.patch");
        fs::write(&good, "--- a/a.txt\n+++ b/a.txt\n@@ -1,2 +1,3 @@\n one\n two\n+three\n").unwrap();
        let half = dir.join("half.patch");
        fs::write(&half, "--- a/b.txt\n+++ b/b.txt\n@@ -1,2 +1,3 @@\n x\n y\n+z\n@@ -40,2 +41,3 @@\n nothing\n like\n+this\n").unwrap();
        let cancel = AtomicBool::new(false);
        let sets = vec![("bore".to_string(), vec![good.clone()]), ("zen".to_string(), vec![half.clone()])];
        let err = apply(&dir, &sets, &cancel, &mut |_, _| {}).unwrap_err();
        assert!(err.contains("No patch was left applied"), "{err}");
        // Nothing was left: not the first one, and not the hunk of the second that fit.
        assert_eq!(fs::read_to_string(dir.join("a.txt")).unwrap(), "one\ntwo\n");
        assert_eq!(fs::read_to_string(dir.join("b.txt")).unwrap(), "x\ny\n");
        assert!(!dir.join(".applied_patches").exists());
        assert!(!dir.join("b.txt.rej").exists());
        let _ = fs::remove_dir_all(&dir);
    }
}
