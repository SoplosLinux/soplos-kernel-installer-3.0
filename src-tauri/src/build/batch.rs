// Building a whole release of the kernels of Soplos, one after another and
// without anybody there: 4 variants for each of the 4 architecture levels, and
// the X3D one for V3 and V4 (18 in all). Each starts from fresh sources, and
// the packages go to `<destination>/<version>/V<level>/` with a metapackage
// that pulls the image and the headers. If one fails the queue stops, and its
// sources and log are kept. Ported from `core/batch.py` of the 2.x program.

use super::pipeline::{self, build_dir, check_cancel, state, Ctx, Options};
use super::recipe::Recipe;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use tauri::AppHandle;

const MARCHES: [&str; 4] = ["v1", "v2", "v3", "v4"];
// The patches of each variant, in the order of the name. BORE alone and NTSYNC
// alone are not built any more: they are kernels nobody uses now.
const VARIANTS: [&[&str]; 4] = [&[], &["bore", "ntsync"], &["rt"], &["zen"]];
// X3D carries BORE and NTSYNC, and only makes sense from Zen 3 on.
const X3D: [&str; 3] = ["bore", "ntsync", "x3d"];
const NAME_ORDER: [&str; 5] = ["bore", "rt", "zen", "ntsync", "x3d"];

#[derive(Serialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct BatchJob {
    pub name: String,
    pub patches: Vec<String>,
    pub march: String,
}

/// "soplos-bore-ntsync-v3". X3D is always "soplos-x3d-v3". `channel` is "rc",
/// "lts" or "eol" for a kernel of that channel, empty for the regular stable
/// line: those must never share a package name with the stable line, or a
/// release candidate could replace a stable kernel on a plain upgrade.
pub fn kernel_name(patches: &[&str], march: &str, channel: &str) -> String {
    let mut ordered: Vec<&str> = patches.to_vec();
    ordered.sort_by_key(|p| NAME_ORDER.iter().position(|o| o == p).unwrap_or(99));
    let mut parts: Vec<&str> = vec!["soplos"];
    if !channel.is_empty() {
        parts.push(channel);
    }
    if ordered.contains(&"x3d") {
        parts.push("x3d");
    } else {
        parts.extend(ordered);
    }
    parts.push(march);
    parts.join("-")
}

pub fn release_queue(channel: &str) -> Vec<BatchJob> {
    let mut jobs = Vec::new();
    let job = |patches: &[&str], march: &str| BatchJob { name: kernel_name(patches, march, channel), patches: patches.iter().map(|p| p.to_string()).collect(), march: march.to_string() };
    for march in MARCHES {
        for variant in VARIANTS {
            jobs.push(job(variant, march));
        }
        if march == "v3" || march == "v4" {
            jobs.push(job(&X3D, march));
        }
    }
    jobs
}

/// The label of the metapackage: "Stock", "BORE V3", "Bore Ntsync V3".
pub fn display_name(pkg: &str) -> String {
    let suffix = pkg.strip_prefix("linux-soplos").unwrap_or("");
    if suffix.is_empty() {
        return "Stock".to_string();
    }
    let parts: Vec<&str> = suffix.trim_start_matches('-').split('-').collect();
    let march = parts.last().filter(|p| MARCHES.contains(p)).map(|m| m.to_uppercase());
    let flavor: Vec<&str> = if march.is_some() { parts[..parts.len() - 1].to_vec() } else { parts.clone() };
    let label = if flavor.is_empty() {
        "Stock".to_string()
    } else if flavor.join("-").len() <= 5 {
        flavor.join("-").to_uppercase()
    } else {
        flavor.iter().map(|w| w[..1].to_uppercase() + &w[1..]).collect::<Vec<_>>().join(" ")
    };
    match march {
        Some(m) => format!("{label} {m}"),
        None => label,
    }
}

/// The Debian revision of the packages. `revision` is the counter of the user:
/// 0 is a kernel version never built, and 1 the first recompile of it. The
/// recompiles are offset by one so the first one never collides with the
/// implicit revision 1 of the original build, and apt sees a real upgrade.
pub fn kdeb(revision: u32) -> u32 {
    revision + 1
}

/// The version of the metapackage: plain on the first build, with `-N` once the
/// same kernel version is recompiled.
pub fn versioned(version: &str, revision: u32) -> String {
    if revision == 0 {
        version.to_string()
    } else {
        format!("{version}-{}", kdeb(revision))
    }
}

pub fn dest_dir(root: &Path, version: &str, march: &str, revision: u32) -> PathBuf {
    let mut dir = root.join(version).join(march.to_uppercase());
    if revision > 0 {
        dir = dir.join(format!("rev{revision}"));
    }
    dir
}

pub fn metapackage_file(name: &str, version: &str, revision: u32) -> String {
    format!("linux-{name}_{}_amd64.deb", versioned(version, revision))
}

/// A kernel is built when its metapackage is in the destination.
pub fn already_built(root: &Path, version: &str, job: &BatchJob, revision: u32) -> bool {
    dest_dir(root, version, &job.march, revision).join(metapackage_file(&job.name, version, revision)).is_file()
}

pub fn control_text(name: &str, version: &str, revision: u32, release: &str) -> String {
    let pkg = format!("linux-{name}");
    format!(
        "Package: {pkg}\nVersion: {}\nArchitecture: amd64\nMaintainer: Soplos Linux Team <info@soplos.org>\nDepends: linux-image-{release}, linux-headers-{release}\nSection: kernel\nPriority: optional\nDescription: Soplos {} kernel\n Installs the {release} kernel and its headers.\n When a new version is released, upgrading this package will\n automatically pull in the new kernel.\n",
        versioned(version, revision),
        display_name(&pkg)
    )
}

#[derive(Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct BatchSpec {
    pub version: String,
    /// "", "rc", "lts" or "eol".
    pub channel: String,
    pub dest: String,
    pub revision: u32,
    pub cores: u32,
    /// Skip the kernels that are already in the destination.
    pub resume: bool,
    /// The names of the kernels to build.
    pub jobs: Vec<String>,
}

fn resolve(spec: &BatchSpec) -> Result<Vec<BatchJob>, String> {
    if !super::recipe::valid_version(&spec.version) {
        return Err(format!("Not a kernel version: {}", spec.version));
    }
    if !["", "rc", "lts", "eol"].contains(&spec.channel.as_str()) {
        return Err(format!("Unknown channel: {}", spec.channel));
    }
    if spec.revision > 98 {
        return Err("The revision is not valid".to_string());
    }
    if spec.cores == 0 || spec.cores > 512 {
        return Err("The number of cores is not valid".to_string());
    }
    if !Path::new(&spec.dest).is_absolute() {
        return Err("The destination has to be a full path".to_string());
    }
    let queue = release_queue(&spec.channel);
    let mut jobs = Vec::new();
    for name in &spec.jobs {
        jobs.push(queue.iter().find(|j| &j.name == name).cloned().ok_or_else(|| format!("Not a kernel of the queue: {name}"))?);
    }
    if jobs.is_empty() {
        return Err("Select at least one kernel".to_string());
    }
    Ok(jobs)
}

// The packages of the kernel that was just built, without the debug ones.
fn collect_debs(builds: &Path, release: &str) -> Vec<PathBuf> {
    let Ok(entries) = fs::read_dir(builds) else { return Vec::new() };
    let mut debs: Vec<PathBuf> = entries
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| {
            let name = p.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
            name.ends_with(".deb") && !name.contains("-dbg") && (name.starts_with(&format!("linux-image-{release}_")) || name.starts_with(&format!("linux-headers-{release}_")))
        })
        .collect();
    debs.sort();
    debs
}

fn make_metapackage(builds: &Path, job: &BatchJob, version: &str, revision: u32, release: &str) -> Result<PathBuf, String> {
    let tmp = std::env::temp_dir().join(format!("ki-meta-{}-{}", std::process::id(), job.name));
    let _ = fs::remove_dir_all(&tmp);
    fs::create_dir_all(tmp.join("DEBIAN")).map_err(|e| e.to_string())?;
    fs::write(tmp.join("DEBIAN/control"), control_text(&job.name, version, revision, release)).map_err(|e| e.to_string())?;
    let out = builds.join(metapackage_file(&job.name, version, revision));
    let status = Command::new("dpkg-deb").args(["--root-owner-group", "--build"]).arg(&tmp).arg(&out).output().map_err(|e| e.to_string())?;
    let _ = fs::remove_dir_all(&tmp);
    if status.status.success() && out.is_file() {
        Ok(out)
    } else {
        Err("The metapackage could not be made".to_string())
    }
}

/// Copies the packages of a finished build to their folder. Returns the folder.
fn save_packages(builds: &Path, root: &Path, job: &BatchJob, version: &str, revision: u32, release: &str) -> Result<PathBuf, String> {
    let mut debs = collect_debs(builds, release);
    if debs.is_empty() {
        return Err(format!("No packages were found for {}", job.name));
    }
    debs.push(make_metapackage(builds, job, version, revision, release)?);
    let dir = dest_dir(root, version, &job.march, revision);
    fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    for deb in &debs {
        let name = deb.file_name().ok_or("A package has no name")?;
        fs::copy(deb, dir.join(name)).map_err(|e| format!("{}: {e}", deb.display()))?;
    }
    Ok(dir)
}

// Nothing of the kernel before is left for the next one.
fn clean(builds: &Path, version: &str) {
    let _ = fs::remove_dir_all(builds.join(format!("linux-{version}")));
    let _ = fs::remove_dir_all(builds.join("patches"));
    if let Ok(entries) = fs::read_dir(builds) {
        for e in entries.filter_map(|e| e.ok()) {
            if e.file_name().to_string_lossy().ends_with(".deb") {
                let _ = fs::remove_file(e.path());
            }
        }
    }
}

fn progress(index: u32, total: u32, name: &str, completed: &[String], dest: &str, failed: Option<String>) {
    if let Ok(mut s) = state().lock() {
        s.percent = 0.0;
        s.batch = Some(pipeline::BatchProgress { index, total, name: name.to_string(), completed: completed.to_vec(), dest: dest.to_string(), failed });
    }
}

fn run(ctx: &Ctx, spec: &BatchSpec) -> Result<String, String> {
    let mut jobs = resolve(spec)?;
    let root = PathBuf::from(&spec.dest);
    fs::create_dir_all(&root).map_err(|e| format!("{}: {e}", root.display()))?;
    if spec.resume {
        jobs.retain(|j| !already_built(&root, &spec.version, j, spec.revision));
        if jobs.is_empty() {
            return Err("Every selected kernel is already built in the destination".to_string());
        }
    }
    let builds = build_dir();
    let total = jobs.len() as u32;

    // The tools are looked for once: the whole queue asks for the password once.
    pipeline::ensure_deps(ctx)?;

    let mut completed: Vec<String> = Vec::new();
    for (i, job) in jobs.iter().enumerate() {
        check_cancel()?;
        let index = i as u32 + 1;
        clean(&builds, &spec.version);
        pipeline::reset_log();
        progress(index, total, &job.name, &completed, &spec.dest, None);
        ctx.info("build", 0.0, &format!("[{index}/{total}] {}", job.name));

        let recipe = Recipe {
            version: spec.version.clone(),
            goal: "stock".to_string(),
            patches: job.patches.clone(),
            march: job.march.clone(),
            name: job.name.clone(),
            sign: false,
            cores: spec.cores,
            reuse_sources: false,
            sched_ext: true,
        };
        let options = Options { kdeb: kdeb(spec.revision), keep_tarball: true, check_deps: false };
        let release = match pipeline::build_kernel(ctx, &recipe, &options) {
            Ok(r) => r,
            Err(e) if e == "cancelled" => return Err(e),
            Err(e) => {
                // Its sources and its log stay for whoever looks at them.
                progress(index, total, &job.name, &completed, &spec.dest, Some(builds.to_string_lossy().to_string()));
                return Err(format!("The build of {} failed: {e}", job.name));
            }
        };
        let saved = match save_packages(&builds, &root, job, &spec.version, spec.revision, &release) {
            Ok(dir) => dir,
            Err(e) => {
                progress(index, total, &job.name, &completed, &spec.dest, Some(builds.to_string_lossy().to_string()));
                return Err(e);
            }
        };
        completed.push(job.name.clone());
        progress(index, total, &job.name, &completed, &spec.dest, None);
        ctx.info("done", 100.0, &format!("[{index}/{total}] {} saved in {}", job.name, saved.display()));
        clean(&builds, &spec.version);
    }
    Ok(spec.dest.clone())
}

pub fn start(app: AppHandle, spec: BatchSpec) -> Result<(), String> {
    resolve(&spec)?;
    let initial = pipeline::BatchProgress { index: 0, total: spec.jobs.len() as u32, name: String::new(), completed: Vec::new(), dest: spec.dest.clone(), failed: None };
    pipeline::spawn(app, None, Some(initial), move |ctx| run(ctx, &spec))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names() {
        assert_eq!(kernel_name(&[], "v1", ""), "soplos-v1");
        assert_eq!(kernel_name(&["bore", "ntsync"], "v3", ""), "soplos-bore-ntsync-v3");
        assert_eq!(kernel_name(&["ntsync", "bore"], "v3", ""), "soplos-bore-ntsync-v3");
        assert_eq!(kernel_name(&["bore", "ntsync", "x3d"], "v4", ""), "soplos-x3d-v4");
        assert_eq!(kernel_name(&["rt"], "v2", "rc"), "soplos-rc-rt-v2");
        assert_eq!(kernel_name(&["bore", "ntsync", "x3d"], "v3", "lts"), "soplos-lts-x3d-v3");
    }

    #[test]
    fn the_queue_has_18_kernels() {
        let q = release_queue("");
        assert_eq!(q.len(), 18);
        for gone in ["soplos-bore-v1", "soplos-ntsync-v1", "soplos-bore-v3", "soplos-ntsync-v4"] {
            assert!(!q.iter().any(|j| j.name == gone), "{gone} is not built any more");
        }
        assert_eq!(q[0].name, "soplos-v1");
        assert_eq!(q.iter().filter(|j| j.patches.contains(&"x3d".to_string())).count(), 2);
        assert!(q.iter().filter(|j| j.patches.contains(&"x3d".to_string())).all(|j| j.march == "v3" || j.march == "v4"));
        let names: std::collections::HashSet<&str> = q.iter().map(|j| j.name.as_str()).collect();
        assert_eq!(names.len(), 18, "the names do not repeat");
        assert_eq!(release_queue("rc").len(), 18);
    }

    #[test]
    fn labels_of_the_metapackage() {
        assert_eq!(display_name("linux-soplos"), "Stock");
        assert_eq!(display_name("linux-soplos-v3"), "Stock V3");
        assert_eq!(display_name("linux-soplos-bore-v4"), "BORE V4");
        assert_eq!(display_name("linux-soplos-bore-ntsync-v3"), "Bore Ntsync V3");
        assert_eq!(display_name("linux-soplos-x3d-v3"), "X3D V3");
    }

    #[test]
    fn revisions() {
        assert_eq!(kdeb(0), 1);
        assert_eq!(kdeb(1), 2);
        assert_eq!(versioned("7.3.2", 0), "7.3.2");
        assert_eq!(versioned("7.3.2", 1), "7.3.2-2");
        assert_eq!(dest_dir(Path::new("/d"), "7.3.2", "v3", 0), PathBuf::from("/d/7.3.2/V3"));
        assert_eq!(dest_dir(Path::new("/d"), "7.3.2", "v3", 2), PathBuf::from("/d/7.3.2/V3/rev2"));
        assert_eq!(metapackage_file("soplos-bore-v3", "7.3.2", 1), "linux-soplos-bore-v3_7.3.2-2_amd64.deb");
    }

    #[test]
    fn the_metapackage_depends_on_the_kernel() {
        let c = control_text("soplos-bore-v3", "7.3.2", 0, "7.3.2-soplos-bore-v3");
        assert!(c.contains("Package: linux-soplos-bore-v3\n"));
        assert!(c.contains("Version: 7.3.2\n"));
        assert!(c.contains("Depends: linux-image-7.3.2-soplos-bore-v3, linux-headers-7.3.2-soplos-bore-v3\n"));
        assert!(c.contains("Description: Soplos BORE V3 kernel\n"));
    }

    #[test]
    fn what_is_already_built_is_found() {
        let root = std::env::temp_dir().join(format!("ki-batch-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let job = release_queue("").into_iter().find(|j| j.name == "soplos-bore-ntsync-v3").unwrap();
        assert!(!already_built(&root, "7.3.2", &job, 0));
        let dir = dest_dir(&root, "7.3.2", "v3", 0);
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("linux-soplos-bore-ntsync-v3_7.3.2_amd64.deb"), "").unwrap();
        assert!(already_built(&root, "7.3.2", &job, 0));
        assert!(!already_built(&root, "7.3.2", &job, 1));
        let _ = fs::remove_dir_all(&root);
    }

    fn spec() -> BatchSpec {
        BatchSpec { version: "7.3.2".into(), channel: "".into(), dest: "/tmp/x".into(), revision: 0, cores: 4, resume: false, jobs: vec!["soplos-v1".into(), "soplos-x3d-v3".into()] }
    }

    #[test]
    fn the_request_is_checked() {
        assert_eq!(resolve(&spec()).unwrap().len(), 2);
        let mut s = spec();
        s.jobs = vec!["soplos-x3d-v1".into()];
        assert!(resolve(&s).is_err());
        let mut s = spec();
        s.jobs.clear();
        assert!(resolve(&s).is_err());
        let mut s = spec();
        s.dest = "relative/path".into();
        assert!(resolve(&s).is_err());
        let mut s = spec();
        s.channel = "evil".into();
        assert!(resolve(&s).is_err());
        let mut s = spec();
        s.version = "7.3;rm".into();
        assert!(resolve(&s).is_err());
        let mut s = spec();
        s.revision = 99;
        assert!(resolve(&s).is_err());
    }

    #[test]
    fn packages_are_saved_with_their_metapackage() {
        let root = std::env::temp_dir().join(format!("ki-batch-save-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let builds = root.join("builds");
        fs::create_dir_all(&builds).unwrap();
        let release = "7.3.2-soplos-v1";
        for n in ["linux-image-7.3.2-soplos-v1_1_amd64.deb", "linux-headers-7.3.2-soplos-v1_1_amd64.deb", "linux-image-7.3.2-soplos-v1-dbg_1_amd64.deb", "linux-libc-dev_7.3.2-1_amd64.deb"] {
            fs::write(builds.join(n), "x").unwrap();
        }
        let job = release_queue("").into_iter().find(|j| j.name == "soplos-v1").unwrap();
        let dir = save_packages(&builds, &root.join("dest"), &job, "7.3.2", 0, release);
        // dpkg-deb makes the metapackage: it is part of any Debian system.
        let dir = dir.expect("the packages are saved");
        let mut names: Vec<String> = fs::read_dir(&dir).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().to_string()).collect();
        names.sort();
        assert_eq!(names, vec!["linux-headers-7.3.2-soplos-v1_1_amd64.deb", "linux-image-7.3.2-soplos-v1_1_amd64.deb", "linux-soplos-v1_7.3.2_amd64.deb"]);
        let _ = fs::remove_dir_all(&root);
    }
}
