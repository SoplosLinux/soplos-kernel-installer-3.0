// The kernels of Debian, XanMod and Liquorix: what the administrator step does
// to install and remove them. XanMod and Liquorix come from repositories of
// their own, so adding one is undone if apt cannot read it: a repository
// that fails would break every `apt update` of the computer.

use crate::commands::hardware::psabi_level;
use std::fs;
use std::io::Write;
use std::process::{Command, Stdio};

pub const DEBIAN_KERNELS: [&str; 2] = ["linux-image-amd64", "linux-image-rt-amd64"];
pub const XANMOD_VARIANTS: [&str; 4] = ["xanmod-main", "xanmod-rt", "xanmod-edge", "xanmod-lts"];

const XANMOD_LIST: &str = "/etc/apt/sources.list.d/xanmod-release.list";
const XANMOD_KEY: &str = "/etc/apt/keyrings/xanmod-archive-keyring.gpg";
const XANMOD_KEY_URL: &str = "https://dl.xanmod.org/archive.key";
const LIQUORIX_LIST: &str = "/etc/apt/sources.list.d/liquorix.list";
const LIQUORIX_KEY: &str = "/etc/apt/keyrings/liquorix-keyring.gpg";
const LIQUORIX_KEY_URL: &str = "https://liquorix.net/liquorix-keyring.gpg";
const LIQUORIX_PACKAGES: [&str; 2] = ["linux-image-liquorix-amd64", "linux-headers-liquorix-amd64"];

/// The XanMod package of a variant for this processor, or None when the
/// processor is too old for it.
pub fn xanmod_package(variant: &str, level: u8) -> Option<String> {
    let tier = level.min(3);
    if variant == "xanmod-lts" {
        return Some(format!("linux-xanmod-lts-x64v{tier}"));
    }
    if tier < 2 {
        return None;
    }
    let prefix = match variant {
        "xanmod-main" => "linux-xanmod",
        "xanmod-rt" => "linux-xanmod-rt",
        "xanmod-edge" => "linux-xanmod-edge",
        _ => return None,
    };
    Some(format!("{prefix}-x64v{tier}"))
}

fn env(cmd: &mut Command) -> &mut Command {
    cmd.env("LC_ALL", "C").env("DEBIAN_FRONTEND", "noninteractive").env("PATH", "/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin")
}

fn run(cmd: &mut Command, what: &str) -> Result<(), String> {
    let status = env(cmd).status().map_err(|e| format!("{what}: {e}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("{what} failed"))
    }
}

fn say(text: &str) {
    println!("{text}");
    let _ = std::io::stdout().flush();
}

fn have(program: &str) -> bool {
    Command::new("sh").args(["-c", &format!("command -v {program}")]).output().map(|o| o.status.success()).unwrap_or(false)
}

fn running_release() -> String {
    fs::read_to_string("/proc/sys/kernel/osrelease").unwrap_or_default().trim().to_string()
}

/// The Debian codename the repositories of this computer follow: the most
/// common `n=` of the lines of `apt-cache policy` that come from Debian.
fn debian_codename() -> Option<String> {
    let out = env(Command::new("apt-cache").arg("policy")).output().ok()?;
    most_common_codename(&String::from_utf8_lossy(&out.stdout))
}

pub fn most_common_codename(policy: &str) -> Option<String> {
    let mut counts: Vec<(String, usize)> = Vec::new();
    for line in policy.lines().filter(|l| l.contains("o=Debian")) {
        if let Some(rest) = line.split(',').find_map(|part| part.trim().strip_prefix("n=").map(str::to_string)).or_else(|| {
            line.find("n=").map(|i| line[i + 2..].chars().take_while(|c| c.is_ascii_lowercase()).collect::<String>())
        }) {
            let name: String = rest.chars().take_while(|c| c.is_ascii_lowercase()).collect();
            if name.is_empty() {
                continue;
            }
            match counts.iter_mut().find(|(n, _)| *n == name) {
                Some((_, c)) => *c += 1,
                None => counts.push((name, 1)),
            }
        }
    }
    counts.into_iter().max_by_key(|(_, c)| *c).map(|(n, _)| n)
}

// Downloads a key and writes it in the format apt reads.
fn install_key(url: &str, dest: &str) -> Result<(), String> {
    fs::create_dir_all("/etc/apt/keyrings").map_err(|e| e.to_string())?;
    let key = env(Command::new("curl").args(["-fsSL", "--max-time", "60", url])).stderr(Stdio::inherit()).output().map_err(|e| format!("curl: {e}"))?;
    if !key.status.success() || key.stdout.is_empty() {
        return Err(format!("The key could not be downloaded from {url}"));
    }
    let mut gpg = env(Command::new("gpg").args(["--batch", "--yes", "--dearmor", "-o", dest]))
        .stdin(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .map_err(|e| format!("gpg: {e}"))?;
    gpg.stdin.take().expect("piped").write_all(&key.stdout).map_err(|e| e.to_string())?;
    if !gpg.wait().map_err(|e| e.to_string())?.success() {
        return Err("The key is not valid".to_string());
    }
    let _ = fs::set_permissions(dest, std::os::unix::fs::PermissionsExt::from_mode(0o644));
    Ok(())
}

// apt update that fails when any repository fails, which is how a repository
// that does not exist is noticed.
fn strict_update() -> Result<(), String> {
    run(Command::new("apt-get").args(["update", "-qq", "-o", "APT::Update::Error-Mode=any"]), "apt-get update")
}

fn undo_repository(list: &str, key: &str) {
    let _ = fs::remove_file(list);
    let _ = fs::remove_file(key);
    let _ = run(Command::new("apt-get").args(["update", "-qq"]), "apt-get update");
}

fn update_grub() {
    if have("update-grub") {
        let _ = run(&mut Command::new("update-grub"), "update-grub");
    }
}

fn installed(package: &str) -> bool {
    env(Command::new("dpkg-query").args(["-W", "-f=${db:Status-Abbrev}", package]))
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).starts_with("ii"))
        .unwrap_or(false)
}

// The installed linux-image and linux-headers packages of a flavor, without
// the ones of the kernel that is running.
fn flavor_packages(keyword: &str) -> Vec<String> {
    let running = running_release();
    let Ok(out) = env(Command::new("dpkg-query").args(["-W", "-f=${db:Status-Abbrev} ${Package}\\n"])).output() else { return Vec::new() };
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter_map(|l| {
            let (status, name) = l.split_once(' ')?;
            let is_kernel = name.starts_with("linux-image-") || name.starts_with("linux-headers-");
            (status.starts_with("ii") && is_kernel && name.contains(keyword) && !name.contains(&running)).then(|| name.to_string())
        })
        .collect()
}

pub fn install_debian(package: &str) -> Result<(), String> {
    if !DEBIAN_KERNELS.contains(&package) {
        return Err(format!("Not a kernel of Debian: {package}"));
    }
    say(&format!("Installing {package}"));
    let _ = run(Command::new("apt-get").args(["update", "-qq"]), "apt-get update");
    run(Command::new("apt-get").args(["install", "-y", package]), "apt-get install")
}

pub fn remove_debian(package: &str) -> Result<(), String> {
    if !DEBIAN_KERNELS.contains(&package) {
        return Err(format!("Not a kernel of Debian: {package}"));
    }
    say(&format!("Removing {package}"));
    // Only the package that follows the newest kernel: the kernels that are
    // installed stay until they are removed one by one from the Installed page.
    run(Command::new("apt-get").args(["purge", "-y", package]), "apt-get purge")
}

pub fn repair_xanmod() -> Result<(), String> {
    say("Repairing the XanMod repository");
    undo_repository(XANMOD_LIST, XANMOD_KEY);
    Ok(())
}

pub fn install_xanmod(variant: &str) -> Result<(), String> {
    let package = xanmod_package(variant, psabi_level()).ok_or("This processor does not support this XanMod kernel")?;
    say(&format!("Installing {package}"));
    let codename = debian_codename().filter(|c| matches!(c.as_str(), "bookworm" | "trixie" | "forky" | "sid")).ok_or(
        "Could not tell the Debian version of this computer, and XanMod publishes one repository for each",
    )?;
    // An earlier version wrote a suite XanMod no longer publishes, which breaks
    // every apt update: reinstalling replaces it.
    let _ = fs::remove_file(XANMOD_LIST);
    install_key(XANMOD_KEY_URL, XANMOD_KEY)?;
    fs::write(XANMOD_LIST, format!("deb [signed-by={XANMOD_KEY}] http://deb.xanmod.org {codename} main\n")).map_err(|e| e.to_string())?;
    if let Err(e) = strict_update() {
        undo_repository(XANMOD_LIST, XANMOD_KEY);
        return Err(format!("{e}: the XanMod repository was removed again"));
    }
    // XanMod is built with LLVM, so DKMS must build its modules with it too.
    // Only for computers that have DKMS modules.
    let dkms_modules = have("dkms") && env(Command::new("dkms").arg("status")).output().map(|o| !o.stdout.is_empty()).unwrap_or(false);
    if dkms_modules {
        say("DKMS modules found: installing the LLVM toolchain they need");
        run(Command::new("apt-get").args(["install", "-y", "clang", "lld", "llvm"]), "apt-get install")?;
    }
    run(Command::new("apt-get").args(["install", "-y", &package]), "apt-get install")
}

pub fn remove_xanmod(variant: &str) -> Result<(), String> {
    let package = xanmod_package(variant, psabi_level()).ok_or("This processor does not support this XanMod kernel")?;
    say(&format!("Removing {package}"));
    let mut packages = vec![package];
    packages.extend(flavor_packages("xanmod"));
    run(Command::new("apt-get").args(["remove", "--purge", "-y"]).args(&packages), "apt-get remove")?;
    update_grub();
    Ok(())
}

pub fn install_liquorix() -> Result<(), String> {
    say("Installing the Liquorix kernel");
    let codename = debian_codename().ok_or("Could not tell the Debian version of this computer")?;
    install_key(LIQUORIX_KEY_URL, LIQUORIX_KEY)?;
    fs::write(LIQUORIX_LIST, format!("deb [arch=amd64 signed-by={LIQUORIX_KEY}] https://liquorix.net/debian {codename} main\n")).map_err(|e| e.to_string())?;
    if let Err(e) = strict_update() {
        undo_repository(LIQUORIX_LIST, LIQUORIX_KEY);
        return Err(format!("{e}: the Liquorix repository was removed again"));
    }
    run(Command::new("apt-get").arg("install").arg("-y").args(LIQUORIX_PACKAGES), "apt-get install")
}

pub fn remove_liquorix() -> Result<(), String> {
    say("Removing the Liquorix kernel");
    let mut packages: Vec<String> = LIQUORIX_PACKAGES.iter().filter(|p| installed(p)).map(|p| p.to_string()).collect();
    packages.extend(flavor_packages("liquorix"));
    if !packages.is_empty() {
        run(Command::new("apt-get").args(["remove", "--purge", "-y"]).args(&packages), "apt-get remove")?;
    }
    undo_repository(LIQUORIX_LIST, LIQUORIX_KEY);
    update_grub();
    Ok(())
}

/// The old repository file that names a suite XanMod no longer publishes.
pub fn xanmod_repo_is_broken() -> bool {
    fs::read_to_string(XANMOD_LIST)
        .map(|t| t.lines().any(|l| l.contains("deb.xanmod.org") && l.split_whitespace().any(|w| w == "releases")))
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn xanmod_packages_follow_the_level() {
        assert_eq!(xanmod_package("xanmod-main", 3).as_deref(), Some("linux-xanmod-x64v3"));
        assert_eq!(xanmod_package("xanmod-main", 4).as_deref(), Some("linux-xanmod-x64v3"));
        assert_eq!(xanmod_package("xanmod-rt", 2).as_deref(), Some("linux-xanmod-rt-x64v2"));
        assert_eq!(xanmod_package("xanmod-edge", 1), None);
        assert_eq!(xanmod_package("xanmod-lts", 1).as_deref(), Some("linux-xanmod-lts-x64v1"));
        assert_eq!(xanmod_package("other", 3), None);
    }

    #[test]
    fn the_codename_is_the_most_common_one() {
        let policy = " 500 http://deb.debian.org/debian forky/main amd64 Packages\n     release o=Debian,a=testing,n=forky,l=Debian,c=main,b=amd64\n 100 http://x/debian trixie/main amd64 Packages\n     release o=Debian,a=stable,n=trixie,l=Debian,c=main,b=amd64\n 500 http://y forky/non-free amd64 Packages\n     release o=Debian,a=testing,n=forky,l=Debian,c=non-free,b=amd64\n 500 http://z stable main\n     release o=Other,n=stable,c=main\n";
        assert_eq!(most_common_codename(policy).as_deref(), Some("forky"));
        assert_eq!(most_common_codename("nothing"), None);
    }
}
