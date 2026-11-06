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
        "dpkg-dev" | "libssl-dev" | "libelf-dev" | "linux-libc-dev" | "libncurses-dev" | "libdw-dev" | "rust-src" | "libclang-dev" | "libc6-dev" | "dwarves" => installed(package),
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
    deps.rustc = version_of("rustc");
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
    fn rust_is_not_optional() {
        for p in ["rustc", "rust-src", "bindgen"] {
            assert!(REQUIRED.contains(&p));
        }
    }
}
