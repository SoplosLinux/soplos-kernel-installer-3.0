// Who signed an EFI binary or a kernel image. Read with `sbverify --list`, which
// names the issuer of every signature of the file.

use std::path::Path;
use std::process::{Command, Stdio};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Signer {
    /// Signed with the key of the user.
    Own,
    Debian,
    Microsoft,
    /// Signed, but by someone the program does not know.
    Other,
    None,
}

impl Signer {
    pub fn as_str(self) -> &'static str {
        match self {
            Signer::Own => "own",
            Signer::Debian => "debian",
            Signer::Microsoft => "microsoft",
            Signer::Other => "other",
            Signer::None => "none",
        }
    }

    /// The firmware and shim of this computer trust it: Debian signs the kernels
    /// and the bootloader of its own packages, and the key of the user is in the
    /// list of keys of shim once it is enrolled.
    pub fn is_trusted(self) -> bool {
        matches!(self, Signer::Own | Signer::Debian)
    }
}

/// Reads the output of `sbverify --list`.
pub fn parse_list(text: &str) -> Signer {
    if text.contains("No signature table present") {
        return Signer::None;
    }
    if text.contains("Debian Secure Boot") {
        return Signer::Debian;
    }
    if text.contains("Microsoft") {
        return Signer::Microsoft;
    }
    if text.contains("signature") && text.contains("issuer") {
        return Signer::Other;
    }
    Signer::None
}

/// The signer of a file. With the certificate of the user, a file that it
/// verifies is told apart as the user's own.
pub fn signer_of(file: &Path, own_cert: Option<&Path>) -> Signer {
    if let Some(cert) = own_cert {
        let own = Command::new("sbverify")
            .arg("--cert")
            .arg(cert)
            .arg(file)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .map(|s| s.success())
            .unwrap_or(false);
        if own {
            return Signer::Own;
        }
    }
    match Command::new("sbverify").arg("--list").arg(file).env("LC_ALL", "C").output() {
        Ok(out) => parse_list(&(String::from_utf8_lossy(&out.stdout).to_string() + &String::from_utf8_lossy(&out.stderr))),
        Err(_) => Signer::None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unsigned_files() {
        assert_eq!(parse_list("No signature table present\n"), Signer::None);
        assert_eq!(parse_list(""), Signer::None);
    }

    #[test]
    fn debian_and_microsoft() {
        let debian = "signature 1\nimage signature issuers:\n - /CN=Debian Secure Boot CA\nimage signature certificates:\n - subject: /CN=Debian Secure Boot Signer 2022 - linux\n";
        assert_eq!(parse_list(debian), Signer::Debian);
        let microsoft = "signature 1\nimage signature issuers:\n - /C=US/O=Microsoft Corporation/CN=Microsoft Corporation UEFI CA 2011\n";
        assert_eq!(parse_list(microsoft), Signer::Microsoft);
    }

    #[test]
    fn someone_else() {
        let other = "signature 1\nimage signature issuers:\n - /CN=Some Other CA\n";
        assert_eq!(parse_list(other), Signer::Other);
    }

    #[test]
    fn trust() {
        assert!(Signer::Own.is_trusted());
        assert!(Signer::Debian.is_trusted());
        assert!(!Signer::Microsoft.is_trusted());
        assert!(!Signer::None.is_trusted());
    }
}
