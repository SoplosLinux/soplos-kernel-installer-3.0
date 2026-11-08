// What it takes to build a kernel on this computer: the packages, and a check
// that the ones installed are whole. Ported from `KernelManager` of the 2.x
// program. Rust is not optional any more: the Binder of Waydroid is the Rust one.

use serde::Serialize;
use std::path::Path;
use std::process::Command;

pub const REQUIRED: [&str; 28] = [
    "make", "gcc", "g++", "flex", "bison", "bc", "curl", "tar", "rsync", "cpio", "kmod", "binutils", "fakeroot", "dpkg-dev", "patch", "xz-utils", "zstd",
    "libssl-dev", "libelf-dev", "linux-libc-dev", "libncurses-dev", "pkg-config", "gettext", "dwarves", "libdw-dev",
    // Rust, for the Binder driver.
    "rustc", "rust-src", "bindgen",
];

/// Packages that may also be reinstalled when the headers of the C library are broken.
pub const EXTRA: [&str; 2] = ["libclang-dev", "libc6-dev"];

pub fn allowed(name: &str) -> bool {
    REQUIRED.contains(&name) || EXTRA.contains(&name)
}

#[derive(Serialize, Clone, Debug, Default)]
#[serde(rename_all = "camelCase")]
pub struct Deps {
    pub missing: Vec<String>,
    /// Installed, but a file it must have is gone.
    pub broken: Vec<String>,
    pub rustc: Option<String>,
    pub bindgen: Option<String>,
}

fn installed(package: &str) -> bool {
    Command::new("dpkg-query")
        .args(["-W", "-f=${db:Status-Abbrev}", package])
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).starts_with("ii"))
        .unwrap_or(false)
}

fn program_present(program: &str) -> bool {
    std::env::var_os("PATH").map(|p| std::env::split_paths(&p).any(|d| d.join(program).is_file())).unwrap_or(false)
}

// A package that gives a program counts as there when the program is.
fn is_there(package: &str) -> bool {
    match package {
        "g++" => program_present("g++") || installed(package),
        "xz-utils" => program_present("xz") || installed(package),
        // Rust counts as there when the package of Debian is, or when the `rustc`
        // of the PATH (the one of rustup, say) finds the source of the standard
        // library. A `rustc` that is only a program on the PATH is not enough:
        // without the source the kernel says Rust is not available.
        "rustc" | "rust-src" => installed(package) || rustc_has_core_source("rustc"),
        "dpkg-dev" | "libssl-dev" | "libelf-dev" | "linux-libc-dev" | "libncurses-dev" | "libdw-dev" | "libclang-dev" | "libc6-dev" | "dwarves" => installed(package),
        other => program_present(other) || installed(other),
    }
}

/// A file that must exist for the package to be whole.
fn sentinel(package: &str) -> Option<&'static str> {
    match package {
        "libssl-dev" => Some("/usr/include/openssl/opensslv.h"),
        // objtool includes <gelf.h>.
        "libelf-dev" => Some("/usr/include/gelf.h"),
        _ => None,
    }
}

fn headers_broken() -> bool {
    let arch = Command::new("gcc").arg("-print-multiarch").output().map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string()).unwrap_or_default();
    let dir = std::env::temp_dir().join(format!("ki-headers-{}", std::process::id()));
    let _ = std::fs::create_dir_all(&dir);
    let file = dir.join("t.c");
    let _ = std::fs::write(&file, "#include <linux/limits.h>\n#include <sys/types.h>\nint main(void) { return 0; }\n");
    let mut cmd = Command::new("gcc");
    cmd.arg("-c").arg(&file).arg("-o").arg("/dev/null");
    if !arch.is_empty() {
        cmd.arg(format!("-I/usr/include/{arch}"));
    }
    let broken = cmd.output().map(|o| !o.status.success()).unwrap_or(true);
    let _ = std::fs::remove_dir_all(&dir);
    broken
}

fn version_of(program: &str) -> Option<String> {
    let out = Command::new(program).arg("--version").output().ok()?;
    String::from_utf8_lossy(&out.stdout).lines().next().map(|l| l.trim().to_string()).filter(|l| !l.is_empty())
}

/// Whether a `rustc` finds the source of the standard library, which Rust in the
/// kernel needs (`make rustavailable` looks for it).
fn rustc_has_core_source(rustc: &str) -> bool {
    let Ok(out) = Command::new(rustc).args(["--print", "sysroot"]).output() else { return false };
    let sysroot = String::from_utf8_lossy(&out.stdout).trim().to_string();
    !sysroot.is_empty() && Path::new(&sysroot).join("lib/rustlib/src/rust/library/core/src/lib.rs").is_file()
}

/// `path` with /usr/bin in front of everything else.
fn path_with_usr_bin_first(path: Option<std::ffi::OsString>) -> Option<std::ffi::OsString> {
    let mut dirs = vec![std::path::PathBuf::from("/usr/bin")];
    if let Some(path) = path {
        dirs.extend(std::env::split_paths(&path));
    }
    std::env::join_paths(dirs).ok()
}

/// The PATH the commands of a build need, when it has to change. Kbuild takes
/// `rustc` from the PATH, and the one of rustup comes before /usr/bin in the PATH
/// of many sessions: its toolchain has no `rust-src` unless it was added, and the
/// kernel then says Rust is not available even with the package `rust-src`
/// installed. When the `rustc` that would be used cannot find the source of the
/// standard library and the one of /usr/bin can, /usr/bin goes first. `None`
/// leaves the PATH as it is.
pub fn build_path() -> Option<std::ffi::OsString> {
    if rustc_has_core_source("rustc") || !Path::new("/usr/bin/rustc").is_file() || !rustc_has_core_source("/usr/bin/rustc") {
        return None;
    }
    path_with_usr_bin_first(std::env::var_os("PATH"))
}

pub fn check() -> Deps {
    let mut deps = Deps::default();
    for package in REQUIRED {
        if !is_there(package) {
            deps.missing.push(package.to_string());
        } else if let Some(file) = sentinel(package) {
            if !Path::new(file).exists() {
                deps.broken.push(package.to_string());
            }
        }
    }
    // libclang is what bindgen needs, and it has no program to look for.
    if !installed("libclang-dev") {
        deps.missing.push("libclang-dev".to_string());
    }
    if headers_broken() {
        for package in ["linux-libc-dev", "libc6-dev"] {
            if !deps.broken.contains(&package.to_string()) && !deps.missing.contains(&package.to_string()) {
                deps.broken.push(package.to_string());
            }
        }
    }
    // The version shown is the one of the `rustc` the build will use.
    deps.rustc = if build_path().is_some() { version_of("/usr/bin/rustc") } else { version_of("rustc") };
    deps.bindgen = version_of("bindgen");
    deps
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_known_packages_are_allowed() {
        assert!(allowed("rustc"));
        assert!(allowed("libclang-dev"));
        assert!(allowed("libc6-dev"));
        assert!(!allowed("firefox"));
        assert!(!allowed("rustc; rm -rf /"));
    }

    #[test]
    fn usr_bin_goes_first_in_the_path() {
        let path = path_with_usr_bin_first(Some("/home/u/.cargo/bin:/usr/local/bin:/usr/bin".into())).unwrap();
        let dirs: Vec<_> = std::env::split_paths(&path).collect();
        assert_eq!(dirs[0], std::path::PathBuf::from("/usr/bin"));
        assert_eq!(dirs[1], std::path::PathBuf::from("/home/u/.cargo/bin"));
        let alone = path_with_usr_bin_first(None).unwrap();
        assert_eq!(std::env::split_paths(&alone).count(), 1);
    }

    #[test]
    fn rust_is_not_optional() {
        for p in ["rustc", "rust-src", "bindgen"] {
            assert!(REQUIRED.contains(&p));
        }
    }
}
