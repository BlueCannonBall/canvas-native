//! Windows browser discovery for a GUI-subsystem parent.
//!
//! thirtyfour 0.37's default Windows version probe starts visible console
//! processes, which can exit without output when launched from our GUI binary.
//! Supply the installed browser/version explicitly and keep the probe hidden.
use std::os::windows::process::CommandExt;
use std::{
    io,
    path::{Path, PathBuf},
    process::{Command, Stdio},
};

fn chrome_candidates() -> Vec<PathBuf> {
    let mut paths = Vec::new();
    for root in [
        "ProgramW6432",
        "ProgramFiles",
        "ProgramFiles(x86)",
        "LOCALAPPDATA",
    ] {
        if let Some(root) = std::env::var_os(root) {
            let path = PathBuf::from(root).join("Google/Chrome/Application/chrome.exe");
            if !paths.contains(&path) {
                paths.push(path);
            }
        }
    }
    if let Some(path) = std::env::var_os("PATH") {
        paths.extend(std::env::split_paths(&path).map(|dir| dir.join("chrome.exe")));
    }
    paths
}

fn parse_version(output: &[u8]) -> io::Result<String> {
    let version = std::str::from_utf8(output)
        .map_err(|_| io::Error::other("Chrome version was not UTF-8"))?
        .trim();
    let components: Vec<_> = version.split('.').collect();
    if components.len() != 4
        || components.iter().any(|part| {
            part.is_empty()
                || !part.bytes().all(|c| c.is_ascii_digit())
                || part.parse::<u32>().is_err()
        })
    {
        return Err(io::Error::other(
            "Could not read a valid installed Chrome version",
        ));
    }
    Ok(version.to_owned())
}

fn chrome_version(binary: &Path) -> io::Result<String> {
    let system_root = std::env::var_os("SystemRoot")
        .ok_or_else(|| io::Error::other("Windows SystemRoot is unavailable"))?;
    let powershell =
        PathBuf::from(system_root).join("System32/WindowsPowerShell/v1.0/powershell.exe");
    let output = Command::new(powershell)
        .args([
            "-NoProfile",
            "-NonInteractive",
            "-Command",
            "(Get-Item -LiteralPath $env:CANVAS_NATIVE_CHROME_BINARY).VersionInfo.ProductVersion",
        ])
        .env("CANVAS_NATIVE_CHROME_BINARY", binary)
        .stdin(Stdio::null())
        .creation_flags(0x0800_0000) // CREATE_NO_WINDOW, even from a GUI parent.
        .output()?;
    if !output.status.success() {
        return Err(io::Error::other(format!(
            "Could not read Chrome version (PowerShell exited {})",
            output.status
        )));
    }
    parse_version(&output.stdout)
}

pub fn installed_chrome() -> io::Result<(PathBuf, String)> {
    let mut last_error = None;
    for binary in chrome_candidates()
        .into_iter()
        .filter(|path| path.is_file())
    {
        match chrome_version(&binary) {
            Ok(version) => return Ok((binary, version)),
            Err(error) => last_error = Some(error),
        }
    }
    Err(last_error.unwrap_or_else(|| io::Error::new(io::ErrorKind::NotFound,
        "Chrome is not installed in a standard location or on PATH. Install Google Chrome to sign in.")))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn accepts_windows_version_output() {
        assert_eq!(
            parse_version(b"157.0.8103.38\r\n").unwrap(),
            "157.0.8103.38"
        );
    }
    #[test]
    fn rejects_empty_partial_and_untrusted_output() {
        for invalid in [
            "",
            "157",
            "157.0.1",
            "157.0.1.2.3",
            "157..1.2",
            "157.0.1.x",
            "157.0.1.2; command",
            "4294967296.0.1.2",
        ] {
            assert!(parse_version(invalid.as_bytes()).is_err(), "{invalid}");
        }
    }
    #[test]
    fn rejects_a_nonexistent_browser() {
        assert!(chrome_version(Path::new("Z:/canvas-native-missing/chrome.exe")).is_err());
    }
}
