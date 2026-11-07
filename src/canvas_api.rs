//! Canvas REST and browser authentication, adapted from the reference implementation.
use chrono::{Local, Months};
use directories::{ProjectDirs, UserDirs};
use reqwest::header::{ACCEPT, CONTENT_DISPOSITION, CONTENT_TYPE, COOKIE, LINK, USER_AGENT};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use std::collections::HashSet;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;
use thirtyfour::extensions::query::ElementPollerWithTimeout;
use thirtyfour::prelude::*;
use tokio::sync::{Mutex, RwLock};

const HTTP_USER_AGENT: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/140.0.0.0 Safari/537.36";

fn config_dir() -> anyhow::Result<std::path::PathBuf> {
    let project = ProjectDirs::from("", "", "canvas-native")
        .ok_or_else(|| anyhow::anyhow!("could not locate the system configuration directory"))?;
    Ok(project.config_dir().to_path_buf())
}

#[cfg(test)]
fn parse_canvas_url(value: &str) -> anyhow::Result<reqwest::Url> {
    let url = reqwest::Url::parse(value)?;
    if !matches!(url.scheme(), "http" | "https") || url.host_str().is_none() || url.path() != "/" {
        anyhow::bail!("Canvas URL must be an HTTP(S) origin without a path");
    }
    Ok(url)
}

fn chrome_profile_dir() -> anyhow::Result<std::path::PathBuf> {
    Ok(config_dir()?.join("chrome-profile"))
}

fn canvas_browser_target(canvas_origin: &str, url: &str) -> anyhow::Result<reqwest::Url> {
    let origin = reqwest::Url::parse(canvas_origin)?;
    let target = reqwest::Url::parse(url).or_else(|_| origin.join(url))?;
    if !matches!(target.scheme(), "http" | "https")
        || !target.username().is_empty()
        || target.password().is_some()
    {
        anyhow::bail!("Canvas links must use an HTTP(S) URL without credentials");
    }
    if target.scheme() != origin.scheme()
        || target.host_str() != origin.host_str()
        || target.port_or_known_default() != origin.port_or_known_default()
    {
        anyhow::bail!("only Canvas-hosted links open in the Canvas browser profile");
    }
    Ok(target)
}

/// Whether a link belongs to the configured Canvas origin. External links keep
/// using the system browser; Canvas links need the app-owned Chrome profile so
/// LTI tools launch with the existing Canvas session.
pub fn is_canvas_url(canvas_origin: &str, url: &str) -> bool {
    canvas_browser_target(canvas_origin, url).is_ok()
}

fn find_on_path(name: &str) -> Option<std::path::PathBuf> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .map(|directory| directory.join(name))
        .find(|candidate| candidate.is_file())
}

fn browser_candidates() -> Vec<std::path::PathBuf> {
    let mut candidates = Vec::new();
    if let Some(browser) = std::env::var_os("CANVAS_NATIVE_CHROME") {
        candidates.push(browser.into());
    }
    #[cfg(target_os = "linux")]
    {
        for name in [
            "google-chrome",
            "google-chrome-stable",
            "chromium",
            "chromium-browser",
            "brave-browser",
            "microsoft-edge",
        ] {
            if let Some(browser) = find_on_path(name) {
                candidates.push(browser);
            }
        }
    }
    #[cfg(target_os = "macos")]
    {
        for path in [
            "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
            "/Applications/Chromium.app/Contents/MacOS/Chromium",
        ] {
            candidates.push(path.into());
        }
    }
    #[cfg(target_os = "windows")]
    {
        for name in ["chrome.exe", "msedge.exe"] {
            if let Some(browser) = find_on_path(name) {
                candidates.push(browser);
            }
        }
        if let Some(program_files) = std::env::var_os("PROGRAMFILES") {
            candidates.push(
                std::path::PathBuf::from(program_files)
                    .join("Google/Chrome/Application/chrome.exe"),
            );
        }
        if let Some(program_files) = std::env::var_os("PROGRAMFILES(X86)") {
            candidates.push(
                std::path::PathBuf::from(program_files)
                    .join("Google/Chrome/Application/chrome.exe"),
            );
        }
        if let Some(local_app_data) = std::env::var_os("LOCALAPPDATA") {
            candidates.push(
                std::path::PathBuf::from(local_app_data)
                    .join("Google/Chrome/Application/chrome.exe"),
            );
        }
    }
    candidates
}

/// Open a Canvas-hosted link in Chrome using the same profile and cookies as the
/// app's authentication flow. This is required for LTI tools that must be
/// launched through Canvas rather than by opening their direct launch URL.
pub fn open_canvas_url(canvas_origin: &str, url: &str) -> anyhow::Result<()> {
    let target = canvas_browser_target(canvas_origin, url)?;
    let profile = chrome_profile_dir()?;
    std::fs::create_dir_all(&profile)?;
    let browser = browser_candidates()
        .into_iter()
        .find(|candidate| candidate.is_file())
        .ok_or_else(|| {
            anyhow::anyhow!(
                "No Chrome or Chromium was found. Install one or set CANVAS_NATIVE_CHROME."
            )
        })?;
    let mut command = std::process::Command::new(&browser);
    command
        .arg(format!("--user-data-dir={}", profile.display()))
        .arg("--new-window")
        .arg("--no-first-run")
        .arg("--no-default-browser-check")
        .arg(target.as_str())
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    command
        .spawn()
        .map_err(|error| anyhow::anyhow!("Could not open Canvas in Chrome: {error}"))?;
    Ok(())
}

fn next_link(header: Option<&reqwest::header::HeaderValue>) -> anyhow::Result<Option<String>> {
    let Some(header) = header else {
        return Ok(None);
    };
    let header = header.to_str()?;
    for part in header.split(',') {
        if !part.split(';').any(|value| value.trim() == "rel=\"next\"") {
            continue;
        }
        let start = part
            .find('<')
            .ok_or_else(|| anyhow::anyhow!("malformed Canvas Link header"))?
            + 1;
        let end = part[start..]
            .find('>')
            .ok_or_else(|| anyhow::anyhow!("malformed Canvas Link header"))?
            + start;
        return Ok(Some(part[start..end].to_owned()));
    }
    Ok(None)
}

fn canvas_resource_path(canvas_origin: &reqwest::Url, url: &str) -> anyhow::Result<String> {
    let url = if url.starts_with("http://") || url.starts_with("https://") {
        reqwest::Url::parse(url)?
    } else {
        canvas_origin.join(url)?
    };
    if url.scheme() != canvas_origin.scheme()
        || url.host_str() != canvas_origin.host_str()
        || url.port_or_known_default() != canvas_origin.port_or_known_default()
    {
        anyhow::bail!("attachment URL must belong to {canvas_origin}");
    }

    let mut path = url.path().trim_start_matches('/').to_owned();
    if let Some(query) = url.query() {
        path.push('?');
        path.push_str(query);
    }
    if path.is_empty() || path.contains('\\') {
        anyhow::bail!("invalid Canvas attachment URL");
    }
    Ok(path)
}

fn validate_numeric_id(kind: &str, id: &str) -> anyhow::Result<()> {
    if id.is_empty() || !id.chars().all(|value| value.is_ascii_digit()) {
        anyhow::bail!("Canvas {kind} ID must be numeric");
    }
    Ok(())
}

fn percent_decode_str(input: &str) -> String {
    let mut bytes = Vec::with_capacity(input.len());
    let mut iter = input.as_bytes().iter().copied();
    while let Some(b) = iter.next() {
        if b == b'%' {
            let h1 = iter.next();
            let h2 = iter.next();
            if let (Some(h1), Some(h2)) = (h1, h2) {
                let hex_str = [h1, h2];
                if let Ok(s) = std::str::from_utf8(&hex_str)
                    && let Ok(byte) = u8::from_str_radix(s, 16)
                {
                    bytes.push(byte);
                    continue;
                }
                bytes.push(b'%');
                bytes.push(h1);
                bytes.push(h2);
            } else {
                bytes.push(b'%');
                if let Some(h1) = h1 {
                    bytes.push(h1);
                }
            }
        } else {
            bytes.push(b);
        }
    }
    String::from_utf8_lossy(&bytes).into_owned()
}

fn extract_csrf_token(cookie_header: &str) -> Option<String> {
    for pair in cookie_header.split(';') {
        let mut parts = pair.splitn(2, '=');
        let name = parts.next()?.trim();
        let value = parts.next()?.trim();
        if name == "_csrf_token" {
            return Some(percent_decode_str(value));
        }
    }
    None
}

fn attachment_url(canvas_origin: &reqwest::Url, resource: &str) -> anyhow::Result<reqwest::Url> {
    let raw = resource.trim();
    let path = if let Some(stripped) = raw.strip_prefix("canvas://") {
        stripped
    } else if let Some(stripped) = raw.strip_prefix("canvas-text://") {
        stripped
    } else {
        raw
    };
    if path.is_empty() || path.starts_with("//") || path.contains('\\') {
        anyhow::bail!("invalid Canvas resource URI: {resource}");
    }

    // A resource payload can be an absolute URL or a path (with or without a leading slash).
    // Validate the resolved origin before attaching session cookies.
    let url = if path.starts_with("http://") || path.starts_with("https://") {
        reqwest::Url::parse(path)?
    } else {
        let relative = path.strip_prefix('/').unwrap_or(path);
        if relative.is_empty() {
            anyhow::bail!("invalid Canvas resource URI: {resource}");
        }
        canvas_origin.join(relative)?
    };
    canvas_resource_path(canvas_origin, url.as_str())?;
    if !url.username().is_empty() || url.password().is_some() {
        anyhow::bail!("Canvas attachment URL must not contain credentials");
    }
    Ok(url)
}

fn is_windows_reserved_stem(stem: &str) -> bool {
    let upper = stem.to_ascii_uppercase();
    matches!(
        upper.as_str(),
        "CON"
            | "PRN"
            | "AUX"
            | "NUL"
            | "COM0"
            | "COM1"
            | "COM2"
            | "COM3"
            | "COM4"
            | "COM5"
            | "COM6"
            | "COM7"
            | "COM8"
            | "COM9"
            | "LPT0"
            | "LPT1"
            | "LPT2"
            | "LPT3"
            | "LPT4"
            | "LPT5"
            | "LPT6"
            | "LPT7"
            | "LPT8"
            | "LPT9"
    )
}

pub fn sanitize_filename(name: &str) -> String {
    let raw = name.trim().trim_matches('"').trim();
    if raw.is_empty() {
        return "attachment".to_owned();
    }

    // Decode percent-encoding if present (e.g. %2e%2e or %2f)
    let decoded = if raw.contains('%') {
        percent_decode_str(raw)
    } else {
        raw.to_owned()
    };

    // Extract the final path segment across both UNIX and Windows separators
    let last_segment = decoded
        .split(['/', '\\'])
        .rfind(|seg| !seg.is_empty())
        .unwrap_or("attachment");

    // Strict allowlist: only ASCII alphanumeric, spaces, and safe punctuation (_ - .)
    let mut cleaned = String::with_capacity(last_segment.len());
    for c in last_segment.chars() {
        if c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.' | ' ') {
            cleaned.push(c);
        } else {
            cleaned.push('_');
        }
    }

    // Strip leading/trailing whitespace and dots (Windows strips trailing dots/spaces)
    let trimmed = cleaned
        .trim()
        .trim_matches(|c| c == ' ' || c == '.')
        .to_owned();

    // If empty or all dots/underscores (e.g. "." or ".."), fall back to default
    if trimmed.is_empty() || trimmed.chars().all(|c| c == '.' || c == '_') {
        return "attachment".to_owned();
    }

    // Defuse Windows reserved device names (e.g. CON, PRN, AUX, NUL, COM1..9, LPT1..9)
    let stem = trimmed.split('.').next().unwrap_or(&trimmed);
    let safe_device_name = if is_windows_reserved_stem(stem) {
        format!("_{trimmed}")
    } else {
        trimmed
    };

    // Limit length to 200 characters to prevent filesystem ENAMETOOLONG errors, preserving extension
    if safe_device_name.len() > 200 {
        if let Some(dot_idx) = safe_device_name.rfind('.') {
            let ext = &safe_device_name[dot_idx..];
            if ext.len() < 30 {
                let stem_len = 200 - ext.len();
                let truncated: String =
                    safe_device_name[..dot_idx].chars().take(stem_len).collect();
                return format!("{truncated}{ext}");
            }
        }
        let truncated: String = safe_device_name.chars().take(200).collect();
        truncated
    } else {
        safe_device_name
    }
}

fn parse_content_disposition_params(header: &str) -> Vec<(String, String)> {
    let mut params = Vec::new();
    let mut chars = header.chars().peekable();

    // Skip disposition-type (e.g. "attachment" or "inline")
    while let Some(&c) = chars.peek() {
        if c == ';' {
            chars.next();
            break;
        }
        chars.next();
    }

    while chars.peek().is_some() {
        while let Some(&c) = chars.peek() {
            if c.is_whitespace() || c == ';' {
                chars.next();
            } else {
                break;
            }
        }
        if chars.peek().is_none() {
            break;
        }

        let mut key = String::new();
        while let Some(&c) = chars.peek() {
            if c == '=' || c == ';' || c.is_whitespace() {
                break;
            }
            key.push(c);
            chars.next();
        }

        while let Some(&c) = chars.peek() {
            if c.is_whitespace() {
                chars.next();
            } else {
                break;
            }
        }

        if chars.peek() == Some(&'=') {
            chars.next(); // consume '='
            while let Some(&c) = chars.peek() {
                if c.is_whitespace() {
                    chars.next();
                } else {
                    break;
                }
            }

            let mut val = String::new();
            if chars.peek() == Some(&'"') {
                chars.next(); // consume opening quote
                while let Some(c) = chars.next() {
                    if c == '\\' {
                        if let Some(escaped) = chars.next() {
                            val.push(escaped);
                        }
                    } else if c == '"' {
                        break; // closing quote
                    } else {
                        val.push(c);
                    }
                }
            } else {
                while let Some(&c) = chars.peek() {
                    if c == ';' || c.is_whitespace() {
                        break;
                    }
                    val.push(c);
                    chars.next();
                }
            }
            params.push((key.to_ascii_lowercase(), val));
        }
    }
    params
}

pub fn extract_content_disposition_filename(header: &str) -> Option<String> {
    let params = parse_content_disposition_params(header);

    // 1. Check for filename* parameter (RFC 5987 / RFC 6266 precedence)
    for (key, val) in &params {
        if key == "filename*" {
            let encoded = if let Some((_, enc)) = val.split_once("''") {
                enc
            } else if let Some(first_quote) = val.find('\'') {
                if let Some(second_quote) = val[first_quote + 1..].find('\'') {
                    &val[first_quote + 1 + second_quote + 1..]
                } else {
                    val.as_str()
                }
            } else {
                val.as_str()
            };
            let decoded = percent_decode_str(encoded);
            let cleaned = sanitize_filename(&decoded);
            if !cleaned.is_empty() && cleaned != "attachment" {
                return Some(cleaned);
            }
        }
    }

    // 2. Check for filename parameter
    for (key, val) in &params {
        if key == "filename" {
            let cleaned = sanitize_filename(val);
            if !cleaned.is_empty() && cleaned != "attachment" {
                return Some(cleaned);
            }
        }
    }

    None
}

fn default_attachment_filename(mime_type: Option<&str>) -> String {
    let ext = match mime_type.unwrap_or("") {
        "application/pdf" => ".pdf",
        "application/zip" | "application/x-zip-compressed" => ".zip",
        "text/plain" => ".txt",
        "application/json" => ".json",
        "application/xml" | "text/xml" => ".xml",
        "image/png" => ".png",
        "image/jpeg" => ".jpg",
        "image/gif" => ".gif",
        "application/vnd.openxmlformats-officedocument.wordprocessingml.document" => ".docx",
        _ => "",
    };
    format!("attachment{ext}")
}

pub fn extract_file_id_from_resource(resource: &str) -> Option<String> {
    let clean = resource
        .strip_prefix("canvas://")
        .or_else(|| resource.strip_prefix("canvas-text://"))
        .unwrap_or(resource);
    let path = if let Ok(url) = reqwest::Url::parse(clean) {
        url.path().to_string()
    } else {
        clean.to_string()
    };
    let trimmed = path.trim_start_matches('/');
    if let Some(rest) = trimmed.strip_prefix("files/") {
        let id_part: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
        if !id_part.is_empty() {
            return Some(id_part);
        }
    }
    None
}

pub fn safe_attachment_filename(resource: &str, raw_name: &str) -> String {
    let clean = sanitize_filename(raw_name);
    if let Some(file_id) = extract_file_id_from_resource(resource) {
        if clean.starts_with(&format!("{file_id}_")) || clean.starts_with(&format!("{file_id}-")) {
            clean
        } else {
            format!("{file_id}_{clean}")
        }
    } else {
        clean
    }
}

pub fn default_download_dir() -> std::path::PathBuf {
    if let Ok(dir) = std::env::var("CANVAS_DOWNLOAD_DIR")
        && !dir.trim().is_empty()
    {
        return std::path::PathBuf::from(dir.trim());
    }
    if let Some(user_dirs) = UserDirs::new()
        && let Some(download_dir) = user_dirs.download_dir()
    {
        return download_dir.join("Canvas");
    }
    std::env::current_dir()
        .map(|cwd| cwd.join("downloads"))
        .unwrap_or_else(|_| std::path::PathBuf::from("downloads"))
}

pub async fn resolve_and_contain_path(
    destination: Option<&str>,
    filename: &str,
) -> anyhow::Result<std::path::PathBuf> {
    let default_dir = default_download_dir();

    let target_path = match destination {
        Some(dest) if !dest.trim().is_empty() => {
            let trimmed = dest.trim();
            let path = Path::new(trimmed);
            let is_dir = path.is_dir()
                || trimmed.ends_with('/')
                || trimmed.ends_with('\\')
                || (path.extension().is_none() && !path.is_file());
            if is_dir {
                path.join(filename)
            } else {
                path.to_path_buf()
            }
        }
        _ => default_dir.join(filename),
    };

    if let Some(parent) = target_path.parent()
        && !parent.as_os_str().is_empty()
    {
        tokio::fs::create_dir_all(parent).await?;
    }

    let absolute = if target_path.is_absolute() {
        target_path
    } else {
        std::env::current_dir()?.join(&target_path)
    };

    Ok(absolute)
}

fn canvas_api_path(canvas_origin: &reqwest::Url, segments: &[&str]) -> anyhow::Result<String> {
    let mut url = canvas_origin.clone();
    {
        let mut path = url
            .path_segments_mut()
            .map_err(|_| anyhow::anyhow!("Canvas URL cannot contain path segments"))?;
        path.clear();
        path.extend(["api", "v1"]);
        path.extend(segments.iter().copied());
    }
    Ok(url.path().to_owned())
}

// Cancellation must close Chrome before releasing its profile lock. Unlike a
// connector subprocess, the desktop process remains alive after cancelling.
struct BrowserSession {
    driver: Option<WebDriver>,
    profile_lock: Option<std::fs::File>,
}
impl std::ops::Deref for BrowserSession {
    type Target = WebDriver;
    fn deref(&self) -> &WebDriver {
        self.driver.as_ref().expect("active browser session")
    }
}
impl BrowserSession {
    async fn quit(mut self) -> WebDriverResult<()> {
        let result = self
            .driver
            .as_ref()
            .expect("active browser session")
            .clone()
            .quit()
            .await;
        if result.is_ok() {
            self.driver.take();
        }
        result
    }
}
impl Drop for BrowserSession {
    fn drop(&mut self) {
        if let Some(driver) = self.driver.take() {
            let lock = self.profile_lock.take();
            if let Ok(runtime) = tokio::runtime::Handle::try_current() {
                runtime.spawn(async move {
                    let _lock = lock;
                    let _ = tokio::time::timeout(Duration::from_secs(10), driver.quit()).await;
                });
            }
        }
    }
}
async fn canvas_driver(
    canvas_origin: &reqwest::Url,
    headless: bool,
    profile_lock: &std::fs::File,
) -> anyhow::Result<BrowserSession> {
    let profile = chrome_profile_dir()?;
    std::fs::create_dir_all(&profile)?;
    let mut caps = DesiredCapabilities::chrome();
    caps.add_arg(&format!("--user-data-dir={}", profile.display()))?;
    if headless {
        caps.set_headless()?;
    }

    let driver = WebDriver::managed(caps)
        .poller(Arc::new(ElementPollerWithTimeout::new(
            Duration::from_millis(5000),
            Duration::from_millis(500),
        )))
        .await
        .map_err(|error| {
            anyhow::anyhow!(
                "could not start Chrome with Canvas profile {}: {error}. Chrome diagnostics: {}",
                profile.display(),
                profile.join("chrome_debug.log").display()
            )
        })?;
    let driver = BrowserSession {
        driver: Some(driver),
        profile_lock: Some(profile_lock.try_clone()?),
    };
    if let Err(error) = driver.goto(canvas_origin.as_str()).await {
        let _ = driver.quit().await;
        return Err(error.into());
    }
    Ok(driver)
}

// Only browser operations need exclusive access to the shared Chrome profile.
// The OS releases this lock when the file is dropped, including on process exit.
async fn lock_chrome_profile() -> anyhow::Result<std::fs::File> {
    let lock = tokio::task::spawn_blocking(|| -> anyhow::Result<std::fs::File> {
        let profile = chrome_profile_dir()?;
        std::fs::create_dir_all(&profile)?;
        let lock = std::fs::File::options()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(profile.with_extension("lock"))?;
        lock.lock()?;
        Ok(lock)
    })
    .await??;
    // A killed connector releases our file lock, but its ChromeDriver and Chrome
    // children can survive. Recover only after obtaining exclusive profile access.
    recover_orphaned_chrome(&chrome_profile_dir()?).await?;
    Ok(lock)
}

#[cfg(target_os = "linux")]
fn process_parent(pid: u32) -> Option<u32> {
    std::fs::read_to_string(format!("/proc/{pid}/status"))
        .ok()?
        .lines()
        .find_map(|line| line.strip_prefix("PPid:")?.trim().parse().ok())
}

#[cfg(target_os = "linux")]
fn process_is_named(pid: u32, name: &str) -> bool {
    std::fs::read_link(format!("/proc/{pid}/exe"))
        .is_ok_and(|path| path.file_name().is_some_and(|value| value == name))
}

#[cfg(target_os = "linux")]
fn process_is_user_systemd(pid: u32) -> bool {
    if process_parent(pid) != Some(1) {
        return false;
    }
    // systemd --user can be non-dumpable, making /proc/<pid>/exe unreadable
    // even to the same user. Its command line and process name remain readable.
    let Ok(args) = std::fs::read(format!("/proc/{pid}/cmdline")) else {
        return false;
    };
    let mut args = args.split(|byte| *byte == 0);
    let executable = args.next().and_then(|arg| std::str::from_utf8(arg).ok());
    executable.is_some_and(|exe| {
        Path::new(exe)
            .file_name()
            .is_some_and(|name| name == "systemd")
    }) && args.any(|arg| arg == b"--user")
        && std::fs::read_to_string(format!("/proc/{pid}/comm"))
            .is_ok_and(|name| name.trim() == "systemd")
}

#[cfg(target_os = "linux")]
fn orphaned_chromedriver(profile: &Path, browser_pid: u32) -> Option<u16> {
    let browser_args = std::fs::read(format!("/proc/{browser_pid}/cmdline")).ok()?;
    let profile_arg = format!("--user-data-dir={}", profile.display());
    let args: Vec<_> = browser_args.split(|byte| *byte == 0).collect();
    if !args.contains(&profile_arg.as_bytes())
        || !args.contains(&b"--enable-automation".as_slice())
        || args.iter().any(|arg| arg.starts_with(b"--type="))
    {
        return None;
    }
    let driver_pid = process_parent(browser_pid)?;
    if !process_is_named(driver_pid, "chromedriver") {
        return None;
    }
    let parent_pid = process_parent(driver_pid)?;
    // Orphans may be adopted by PID 1 or the user's systemd subreaper. Never
    // touch a driver that is still owned by a live connector/client process.
    if parent_pid != 1 && !process_is_user_systemd(parent_pid) {
        return None;
    }
    let driver_args = std::fs::read(format!("/proc/{driver_pid}/cmdline")).ok()?;
    driver_args.split(|byte| *byte == 0).find_map(|arg| {
        std::str::from_utf8(arg)
            .ok()?
            .strip_prefix("--port=")?
            .parse::<u16>()
            .ok()
            .filter(|port| *port != 0)
    })
}

#[cfg(target_os = "linux")]
async fn close_orphaned_chrome_session(
    endpoint: &str,
    profile: &Path,
    browser_pid: u32,
) -> anyhow::Result<()> {
    let expected_profile = profile
        .to_str()
        .ok_or_else(|| anyhow::anyhow!("Canvas Chrome profile path is not valid UTF-8"))?;
    let client = reqwest::Client::builder()
        .no_proxy()
        .timeout(Duration::from_secs(10))
        .build()?;
    let sessions: serde_json::Value = client
        .get(format!("{endpoint}/sessions"))
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    let session = sessions["value"]
        .as_array()
        .and_then(|sessions| {
            sessions.iter().find(|session| {
                session["capabilities"]["chrome"]["userDataDir"].as_str() == Some(expected_profile)
                    && session["capabilities"]["goog:processID"].as_u64()
                        == Some(u64::from(browser_pid))
            })
        })
        .and_then(|session| session["id"].as_str())
        .ok_or_else(|| anyhow::anyhow!("ChromeDriver has no matching orphaned Canvas session"))?;
    let mut url = reqwest::Url::parse(endpoint)?;
    url.path_segments_mut()
        .map_err(|_| anyhow::anyhow!("invalid ChromeDriver endpoint"))?
        .extend(["session", session]);
    let result: serde_json::Value = client
        .delete(url)
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    if result["value"].get("error").is_some()
        || result["status"].as_u64().is_some_and(|status| status != 0)
    {
        anyhow::bail!("ChromeDriver failed to close the orphaned Canvas session");
    }
    // Deleting the session shuts Chrome down cleanly, preserving its cookies.
    // The now-unused orphaned ChromeDriver should also stop, but a failed driver
    // shutdown must not prevent starting Chrome once the profile is released.
    if sessions["value"]
        .as_array()
        .is_some_and(|sessions| sessions.len() == 1)
    {
        let _ = client.get(format!("{endpoint}/shutdown")).send().await;
    }
    Ok(())
}

#[cfg(target_os = "linux")]
async fn recover_orphaned_chrome(profile: &Path) -> anyhow::Result<()> {
    let Ok(lock) = std::fs::read_link(profile.join("SingletonLock")) else {
        return Ok(());
    };
    let Some((host, pid)) = lock.to_str().and_then(|value| value.rsplit_once('-')) else {
        return Ok(());
    };
    let hostname = std::fs::read_to_string("/proc/sys/kernel/hostname")?;
    if host != hostname.trim() {
        anyhow::bail!(
            "Canvas Chrome profile {} is in use on host {host}",
            profile.display()
        );
    }
    let Ok(browser_pid) = pid.parse::<u32>() else {
        return Ok(());
    };
    if !Path::new(&format!("/proc/{browser_pid}")).exists() {
        // Chrome itself handles stale SingletonLock files whose owner has exited.
        return Ok(());
    }
    let port = orphaned_chromedriver(profile, browser_pid).ok_or_else(|| {
        anyhow::anyhow!(
            "Canvas Chrome profile {} is in use by process {browser_pid}; close that browser before retrying (saved login data will be kept)",
            profile.display()
        )
    })?;
    eprintln!("Closing orphaned Canvas Chrome session (process {browser_pid})");
    close_orphaned_chrome_session(&format!("http://127.0.0.1:{port}"), profile, browser_pid)
        .await
        .map_err(|error| {
            anyhow::anyhow!(
                "could not recover Canvas Chrome profile {}: {error}",
                profile.display()
            )
        })?;
    for _ in 0..50 {
        if std::fs::read_link(profile.join("SingletonLock"))
            .as_ref()
            .ok()
            != Some(&lock)
        {
            return Ok(());
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    anyhow::bail!(
        "orphaned Chrome did not release Canvas profile {}",
        profile.display()
    )
}

#[cfg(not(target_os = "linux"))]
async fn recover_orphaned_chrome(_profile: &Path) -> anyhow::Result<()> {
    Ok(())
}

async fn login(canvas_origin: &reqwest::Url, profile_lock: &std::fs::File) -> anyhow::Result<()> {
    let driver = canvas_driver(canvas_origin, false, profile_lock).await?;

    while driver
        .windows()
        .await
        .is_ok_and(|handles| !handles.is_empty())
    {
        tokio::time::sleep(Duration::from_millis(500)).await;
    }

    let _ = driver.quit().await;
    Ok(())
}

// Caller holds the profile lock until Chrome has finished shutting down.
async fn load_cookie_header(
    canvas_origin: &reqwest::Url,
    profile_lock: &std::fs::File,
) -> anyhow::Result<String> {
    let driver = canvas_driver(canvas_origin, true, profile_lock).await?;
    let cookies = driver.get_all_cookies().await;
    let shutdown = driver.quit().await;
    let cookies = cookies?;
    shutdown?;
    Ok(cookies
        .into_iter()
        .map(|cookie| format!("{}={}", cookie.name, cookie.value))
        .collect::<Vec<_>>()
        .join("; "))
}

fn should_refresh_and_retry(method: &reqwest::Method, status: reqwest::StatusCode) -> bool {
    *method == reqwest::Method::GET && status == reqwest::StatusCode::UNAUTHORIZED
}

#[derive(Clone)]
pub struct CanvasApi {
    canvas_origin: reqwest::Url,
    cookie_header: Arc<RwLock<String>>,
    cookie_reload: Arc<Mutex<()>>,
    client: reqwest::Client,
}

impl CanvasApi {
    pub fn with_origin(canvas_origin: reqwest::Url) -> anyhow::Result<Self> {
        let client = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(10))
            .timeout(Duration::from_secs(60))
            .build()?;
        // Operation discovery must never depend on Chrome, the shared profile
        // lock, the network, or a valid authenticated session.
        Ok(Self {
            canvas_origin,
            cookie_header: Arc::new(RwLock::new(String::new())),
            cookie_reload: Arc::new(Mutex::new(())),
            client,
        })
    }

    async fn canvas_cookie_header(&self) -> anyhow::Result<String> {
        let cookie_header = self.cookie_header.read().await.clone();
        if cookie_header.is_empty() {
            return self
                .reload_cookie_header_if_current(Some(&cookie_header))
                .await;
        }
        Ok(cookie_header)
    }

    async fn replace_cached_cookie_header(&self, cookies: String) -> anyhow::Result<String> {
        if cookies.trim().is_empty() {
            anyhow::bail!(
                "no Canvas cookies are available in the saved browser profile; interactive authentication is required"
            );
        }
        *self.cookie_header.write().await = cookies.clone();
        Ok(cookies)
    }

    async fn reload_cookie_header(&self) -> anyhow::Result<String> {
        self.reload_cookie_header_if_current(None).await
    }

    async fn reload_cookie_header_if_current(
        &self,
        expected: Option<&str>,
    ) -> anyhow::Result<String> {
        self.replace_cookie_header_if_current(expected, async {
            let _profile_lock = lock_chrome_profile().await?;
            load_cookie_header(&self.canvas_origin, &_profile_lock).await
        })
        .await
    }

    async fn replace_cookie_header_if_current(
        &self,
        expected: Option<&str>,
        load: impl std::future::Future<Output = anyhow::Result<String>>,
    ) -> anyhow::Result<String> {
        // Serialize within this process, then recheck: another request may have
        // already loaded/refreshed the cookies while we waited. Do not cache
        // errors, so a later refresh or interactive login can recover.
        let _reload = self.cookie_reload.lock().await;
        let current = self.cookie_header.read().await.clone();
        if expected.is_some_and(|expected| current != expected) && !current.is_empty() {
            return Ok(current);
        }
        self.replace_cached_cookie_header(load.await?).await
    }

    pub async fn authenticate(&self) -> anyhow::Result<AuthenticatedUser> {
        {
            let _reload = self.cookie_reload.lock().await;
            let _profile_lock = lock_chrome_profile().await?;
            login(&self.canvas_origin, &_profile_lock).await?;
            let cookies = load_cookie_header(&self.canvas_origin, &_profile_lock).await?;
            self.replace_cached_cookie_header(cookies).await?;
        }
        self.api_get("/api/v1/users/self/profile", &[]).await
    }

    pub async fn refresh_authentication(&self) -> anyhow::Result<AuthenticatedUser> {
        self.reload_cookie_header().await?;
        self.api_get("/api/v1/users/self/profile", &[]).await
    }

    async fn send_api_request(
        &self,
        method: reqwest::Method,
        url: reqwest::Url,
        body: Option<&serde_json::Value>,
        cookie: &str,
    ) -> anyhow::Result<reqwest::Response> {
        let mut request = self
            .client
            .request(method.clone(), url)
            .header(USER_AGENT, HTTP_USER_AGENT)
            .header(ACCEPT, "application/json");

        if !cookie.is_empty() {
            request = request.header(COOKIE, cookie);
        }

        if method != reqwest::Method::GET
            && let Some(csrf) = extract_csrf_token(cookie)
            && !csrf.is_empty()
        {
            request = request.header("X-CSRF-Token", csrf);
        }

        if let Some(json_body) = body {
            request = request
                .header(CONTENT_TYPE, "application/json")
                .json(json_body);
        }

        Ok(request.send().await?)
    }

    async fn api_request(
        &self,
        method: reqwest::Method,
        url: reqwest::Url,
        body: Option<serde_json::Value>,
        cookie: &str,
    ) -> anyhow::Result<reqwest::Response> {
        let origin = &self.canvas_origin;
        let is_canvas_api = |url: &reqwest::Url| {
            url.scheme() == origin.scheme()
                && url.host_str() == origin.host_str()
                && url.port_or_known_default() == origin.port_or_known_default()
                && url.path().starts_with("/api/v1/")
        };
        if !is_canvas_api(&url) {
            anyhow::bail!("unexpected Canvas API URL: {url}");
        }

        let mut response = self
            .send_api_request(method.clone(), url.clone(), body.as_ref(), cookie)
            .await?;
        if should_refresh_and_retry(&method, response.status()) {
            let refreshed_cookie = self.reload_cookie_header_if_current(Some(cookie)).await?;
            response = self
                .send_api_request(method, url, body.as_ref(), &refreshed_cookie)
                .await?;
        }
        let response = response.error_for_status()?;
        if !is_canvas_api(response.url()) {
            anyhow::bail!(
                "Canvas API redirected away from Canvas; the browser session may have expired. Direct the user to authenticate using the authentication tool"
            );
        }

        if response.status() != reqwest::StatusCode::NO_CONTENT {
            let content_type = response
                .headers()
                .get(CONTENT_TYPE)
                .and_then(|value| value.to_str().ok())
                .unwrap_or("");
            if !content_type
                .to_ascii_lowercase()
                .starts_with("application/json")
            {
                anyhow::bail!(
                    "Canvas API returned {content_type:?} instead of JSON; the browser session may have expired. Direct the user to authenticate using the authentication tool"
                );
            }
        }
        Ok(response)
    }

    async fn api_response(
        &self,
        url: reqwest::Url,
        cookie: &str,
    ) -> anyhow::Result<reqwest::Response> {
        self.api_request(reqwest::Method::GET, url, None, cookie)
            .await
    }

    async fn api_post<B: Serialize, T: DeserializeOwned>(
        &self,
        path: &str,
        body: &B,
    ) -> anyhow::Result<T> {
        if !path.starts_with("/api/v1/") {
            anyhow::bail!("Canvas API path must begin with /api/v1/");
        }
        let url = self.canvas_origin.join(path)?;
        let cookie_header = self.canvas_cookie_header().await?;
        let json_value = serde_json::to_value(body)?;
        let response = self
            .api_request(reqwest::Method::POST, url, Some(json_value), &cookie_header)
            .await?;
        Ok(serde_json::from_slice(&response.bytes().await?)?)
    }

    async fn api_put<B: Serialize, T: DeserializeOwned>(
        &self,
        path: &str,
        body: &B,
    ) -> anyhow::Result<T> {
        if !path.starts_with("/api/v1/") {
            anyhow::bail!("Canvas API path must begin with /api/v1/");
        }
        let url = self.canvas_origin.join(path)?;
        let cookie_header = self.canvas_cookie_header().await?;
        let json_value = serde_json::to_value(body)?;
        let response = self
            .api_request(reqwest::Method::PUT, url, Some(json_value), &cookie_header)
            .await?;
        Ok(serde_json::from_slice(&response.bytes().await?)?)
    }

    async fn api_delete<T: DeserializeOwned>(&self, path: &str) -> anyhow::Result<T> {
        if !path.starts_with("/api/v1/") {
            anyhow::bail!("Canvas API path must begin with /api/v1/");
        }
        let url = self.canvas_origin.join(path)?;
        let cookie_header = self.canvas_cookie_header().await?;
        let response = self
            .api_request(reqwest::Method::DELETE, url, None, &cookie_header)
            .await?;
        if response.status() == reqwest::StatusCode::NO_CONTENT {
            return Ok(serde_json::from_value(serde_json::Value::Null)?);
        }
        let bytes = response.bytes().await?;
        if bytes.is_empty() {
            return Ok(serde_json::from_value(serde_json::Value::Null)?);
        }
        Ok(serde_json::from_slice(&bytes)?)
    }

    async fn api_get_paginated<T: DeserializeOwned>(
        &self,
        path: &str,
        query: &[(&str, String)],
    ) -> anyhow::Result<Vec<T>> {
        const MAX_PAGES: usize = 100;

        if !path.starts_with("/api/v1/") {
            anyhow::bail!("Canvas API path must begin with /api/v1/");
        }

        let mut next_url = self.canvas_origin.join(path)?;
        next_url
            .query_pairs_mut()
            .extend_pairs(query.iter().map(|(key, value)| (*key, value.as_str())));
        let cookie_header = self.canvas_cookie_header().await?;
        let mut seen = HashSet::new();
        let mut items = Vec::new();

        for _ in 0..MAX_PAGES {
            if !seen.insert(next_url.as_str().to_owned()) {
                anyhow::bail!("Canvas pagination returned a repeated next URL");
            }

            let response = self.api_response(next_url, &cookie_header).await?;
            let next = next_link(response.headers().get(LINK))?;
            let mut page: Vec<T> = serde_json::from_slice(&response.bytes().await?)?;
            items.append(&mut page);

            let Some(next) = next else {
                return Ok(items);
            };
            next_url = reqwest::Url::parse(&next)?;
        }

        anyhow::bail!("Canvas API pagination exceeded {MAX_PAGES} pages")
    }

    async fn api_get<T: DeserializeOwned>(
        &self,
        path: &str,
        query: &[(&str, String)],
    ) -> anyhow::Result<T> {
        if !path.starts_with("/api/v1/") {
            anyhow::bail!("Canvas API path must begin with /api/v1/");
        }

        let mut url = self.canvas_origin.join(path)?;
        url.query_pairs_mut()
            .extend_pairs(query.iter().map(|(key, value)| (*key, value.as_str())));
        let cookie_header = self.canvas_cookie_header().await?;
        let response = self.api_response(url, &cookie_header).await?;
        Ok(serde_json::from_slice(&response.bytes().await?)?)
    }

    pub async fn assignment_info(
        &self,
        course_id: &str,
        assignment_id: &str,
    ) -> anyhow::Result<AssignmentInfo> {
        validate_numeric_id("course", course_id)?;
        validate_numeric_id("assignment", assignment_id)?;
        let mut assignment: AssignmentInfo = self
            .api_get(
                &format!("/api/v1/courses/{course_id}/assignments/{assignment_id}"),
                &[("include[]", "submission".into())],
            )
            .await?;
        assignment.prepare_resources(&self.canvas_origin)?;
        Ok(assignment)
    }

    pub async fn course_info(&self, course_id: &str) -> anyhow::Result<CourseInfo> {
        validate_numeric_id("course", course_id)?;
        self.api_get(
            &format!("/api/v1/courses/{course_id}"),
            &[
                ("include[]", "syllabus_body".into()),
                ("include[]", "term".into()),
                ("include[]", "teachers".into()),
                ("include[]", "total_scores".into()),
            ],
        )
        .await
    }

    pub async fn page_info(&self, course_id: &str, page_url: &str) -> anyhow::Result<PageInfo> {
        validate_numeric_id("course", course_id)?;
        if page_url.is_empty() {
            anyhow::bail!("Canvas page URL must not be empty");
        }
        let path = canvas_api_path(
            &self.canvas_origin,
            &["courses", course_id, "pages", page_url],
        )?;
        self.api_get(&path, &[]).await
    }
    pub async fn course_front_page(&self, course_id: &str) -> anyhow::Result<PageInfo> {
        validate_numeric_id("course", course_id)?;
        self.api_get(&format!("/api/v1/courses/{course_id}/front_page"), &[])
            .await
    }

    pub async fn file_info(&self, file_id: &str) -> anyhow::Result<FileInfo> {
        validate_numeric_id("file", file_id)?;
        let mut file: FileInfo = self
            .api_get(&format!("/api/v1/files/{file_id}"), &[])
            .await?;
        file.prepare_resources(&self.canvas_origin)?;
        Ok(file)
    }

    pub async fn attachment(&self, resource: &str) -> anyhow::Result<AttachmentContents> {
        self.attachment_limited(resource, usize::MAX).await
    }
    async fn attachment_response(&self, url: reqwest::Url) -> anyhow::Result<reqwest::Response> {
        let cookies = self.canvas_cookie_header().await?;
        let send = |cookies: String| {
            self.client
                .get(url.clone())
                .header(USER_AGENT, HTTP_USER_AGENT)
                .header(COOKIE, cookies)
                .send()
        };
        let response = send(cookies.clone()).await?;
        if should_refresh_and_retry(&reqwest::Method::GET, response.status()) {
            drop(response);
            let refreshed = self.reload_cookie_header_if_current(Some(&cookies)).await?;
            return Ok(send(refreshed).await?.error_for_status()?);
        }
        Ok(response.error_for_status()?)
    }
    pub async fn attachment_limited(
        &self,
        resource: &str,
        max_bytes: usize,
    ) -> anyhow::Result<AttachmentContents> {
        let url = attachment_url(&self.canvas_origin, resource)?;
        let mut response = self.attachment_response(url).await?;
        let mime_type = response
            .headers()
            .get(CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .map(|value| value.to_owned());
        let header_filename = response
            .headers()
            .get(CONTENT_DISPOSITION)
            .and_then(|value| value.to_str().ok())
            .and_then(extract_content_disposition_filename);
        if response
            .content_length()
            .is_some_and(|size| size > max_bytes as u64)
        {
            anyhow::bail!("Document exceeds the in-app preview size limit; download it instead.");
        }
        let mut bytes = Vec::new();
        while let Some(chunk) = response.chunk().await? {
            if bytes.len().saturating_add(chunk.len()) > max_bytes {
                anyhow::bail!(
                    "Document exceeds the in-app preview size limit; download it instead."
                );
            }
            bytes.extend_from_slice(&chunk);
        }

        Ok(AttachmentContents {
            bytes,
            mime_type,
            filename: header_filename,
        })
    }

    pub async fn save_attachment_to_disk(
        &self,
        resource: &str,
        bytes: &[u8],
        header_filename: Option<&str>,
        mime_type: Option<&str>,
        destination_path: Option<&str>,
    ) -> anyhow::Result<DownloadedFile> {
        let raw_filename = if let Some(h) = header_filename.filter(|s| !s.trim().is_empty()) {
            h.to_owned()
        } else {
            let url = attachment_url(&self.canvas_origin, resource)?;
            let url_segment = url
                .path_segments()
                .and_then(|mut segs| segs.next_back())
                .filter(|seg| !seg.is_empty() && *seg != "download");
            if let Some(seg) = url_segment {
                percent_decode_str(seg)
            } else {
                default_attachment_filename(mime_type)
            }
        };

        let filename = safe_attachment_filename(resource, &raw_filename);
        let target_path = resolve_and_contain_path(destination_path, &filename).await?;

        tokio::fs::write(&target_path, bytes).await?;

        Ok(DownloadedFile {
            saved_path: target_path.to_string_lossy().into_owned(),
            filename,
            bytes: bytes.len() as u64,
            mime_type: mime_type.map(str::to_owned),
        })
    }

    pub async fn download_attachment(
        &self,
        resource: &str,
        destination_path: Option<&str>,
        filename_override: Option<&str>,
    ) -> anyhow::Result<DownloadedFile> {
        let url = attachment_url(&self.canvas_origin, resource)?;
        let mut response = self.attachment_response(url.clone()).await?;
        let mime_type = response
            .headers()
            .get(CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .map(|value| value.to_owned());

        let header_filename = response
            .headers()
            .get(CONTENT_DISPOSITION)
            .and_then(|value| value.to_str().ok())
            .and_then(extract_content_disposition_filename);

        let raw_filename =
            if let Some(override_name) = filename_override.filter(|s| !s.trim().is_empty()) {
                override_name.to_owned()
            } else if let Some(header_name) = header_filename {
                header_name
            } else {
                let url_segment = url
                    .path_segments()
                    .and_then(|mut segs| segs.next_back())
                    .filter(|seg| !seg.is_empty() && *seg != "download");
                if let Some(seg) = url_segment {
                    percent_decode_str(seg)
                } else {
                    default_attachment_filename(mime_type.as_deref())
                }
            };

        let filename = if filename_override.is_some() {
            sanitize_filename(&raw_filename)
        } else {
            safe_attachment_filename(resource, &raw_filename)
        };

        let target_path = resolve_and_contain_path(destination_path, &filename).await?;

        let mut file = tokio::fs::File::create(&target_path).await?;
        let mut bytes_written: u64 = 0;
        while let Some(chunk) = response.chunk().await? {
            tokio::io::AsyncWriteExt::write_all(&mut file, &chunk).await?;
            bytes_written += chunk.len() as u64;
        }
        tokio::io::AsyncWriteExt::flush(&mut file).await?;

        Ok(DownloadedFile {
            saved_path: target_path.to_string_lossy().into_owned(),
            filename,
            bytes: bytes_written,
            mime_type,
        })
    }

    pub async fn download_file(
        &self,
        file_id: &str,
        destination_path: Option<&str>,
        filename_override: Option<&str>,
    ) -> anyhow::Result<DownloadedFile> {
        validate_numeric_id("file", file_id)?;
        let info = self.file_info(file_id).await?;
        let preferred_filename = filename_override
            .or(info.display_name.as_deref())
            .or(Some(&info.filename));
        self.download_attachment(
            &info.download_resource,
            destination_path,
            preferred_filename,
        )
        .await
    }

    pub async fn dashboard_items(
        &self,
        query: &PlannerItemsQuery,
    ) -> anyhow::Result<Vec<DashboardItem>> {
        let params = planner_params(query, Local::now().date_naive());
        let items: Vec<PlannerItem> = self
            .api_get_paginated("/api/v1/planner/items", &params)
            .await?;
        Ok(items.into_iter().map(Into::into).collect())
    }

    pub async fn course_list(&self) -> anyhow::Result<Vec<CourseSummary>> {
        let params = vec![
            ("include[]", "all_courses".into()),
            ("include[]", "term".into()),
            ("include[]", "teachers".into()),
            ("include[]", "total_scores".into()),
            ("include[]", "concluded".into()),
            ("per_page", "100".into()),
        ];
        let mut courses: Vec<CourseSummary> =
            self.api_get_paginated("/api/v1/courses", &params).await?;
        for course in &mut courses {
            course.prepare_urls(&self.canvas_origin);
        }
        Ok(courses)
    }

    pub async fn modules(&self, course_id: &str) -> anyhow::Result<Vec<Module>> {
        validate_numeric_id("course", course_id)?;
        let params = vec![
            ("include[]", "items".into()),
            ("include[]", "content_details".into()),
            ("per_page", "100".into()),
        ];
        let mut modules: Vec<Module> = self
            .api_get_paginated(&format!("/api/v1/courses/{course_id}/modules"), &params)
            .await?;
        let limit = Arc::new(tokio::sync::Semaphore::new(4));
        let mut requests = tokio::task::JoinSet::new();
        for (index, module) in modules.iter().enumerate() {
            if (module.items.len() as u64) < module.items_count {
                let api = self.clone();
                let limit = limit.clone();
                let path = format!("/api/v1/courses/{course_id}/modules/{}/items", module.id);
                requests.spawn(async move {
                    let _permit = limit.acquire_owned().await?;
                    let items: Vec<ModuleItem> = api
                        .api_get_paginated(
                            &path,
                            &[
                                ("include[]", "content_details".into()),
                                ("per_page", "100".into()),
                            ],
                        )
                        .await?;
                    Ok::<_, anyhow::Error>((index, items))
                });
            }
        }
        while let Some(result) = requests.join_next().await {
            let (index, items) = result??;
            modules[index].items = items;
        }
        Ok(modules)
    }

    pub async fn assignments(&self, course_id: &str) -> anyhow::Result<Vec<AssignmentSummary>> {
        validate_numeric_id("course", course_id)?;
        let params = vec![
            ("include[]", "submission".into()),
            ("order_by", "due_at".into()),
            ("per_page", "100".into()),
        ];
        self.api_get_paginated(&format!("/api/v1/courses/{course_id}/assignments"), &params)
            .await
    }

    pub async fn create_planner_note(
        &self,
        payload: &CreatePlannerNotePayload,
    ) -> anyhow::Result<PlannerNote> {
        if let Some(ref course_id) = payload.course_id
            && !course_id.is_empty()
        {
            validate_numeric_id("course", course_id)?;
        }
        if let Some(ref object_id) = payload.linked_object_id
            && !object_id.is_empty()
        {
            validate_numeric_id("linked_object", object_id)?;
        }
        self.api_post("/api/v1/planner_notes", payload).await
    }

    pub async fn update_planner_note(
        &self,
        note_id: &str,
        payload: &UpdatePlannerNotePayload,
    ) -> anyhow::Result<PlannerNote> {
        validate_numeric_id("planner note", note_id)?;
        if let Some(ref course_id) = payload.course_id
            && !course_id.is_empty()
        {
            validate_numeric_id("course", course_id)?;
        }
        self.api_put(&format!("/api/v1/planner_notes/{note_id}"), payload)
            .await
    }

    pub async fn delete_planner_note(&self, note_id: &str) -> anyhow::Result<serde_json::Value> {
        validate_numeric_id("planner note", note_id)?;
        self.api_delete(&format!("/api/v1/planner_notes/{note_id}"))
            .await
    }

    pub async fn planner_note_info(&self, note_id: &str) -> anyhow::Result<PlannerNote> {
        validate_numeric_id("planner note", note_id)?;
        self.api_get(&format!("/api/v1/planner_notes/{note_id}"), &[])
            .await
    }

    pub async fn course_enrollments(
        &self,
        course_id: &str,
    ) -> anyhow::Result<Vec<CourseEnrollment>> {
        validate_numeric_id("course", course_id)?;
        let params = vec![
            ("user_id", "self".into()),
            ("include[]", "total_scores".into()),
            ("include[]", "current_points".into()),
        ];
        self.api_get_paginated(&format!("/api/v1/courses/{course_id}/enrollments"), &params)
            .await
    }

    pub async fn course_grades(
        &self,
        course_id: &str,
        assignment_id: Option<&str>,
    ) -> anyhow::Result<CourseGradesReport> {
        validate_numeric_id("course", course_id)?;
        if let Some(aid) = assignment_id {
            validate_numeric_id("assignment", aid)?;
        }

        let course_fut = self.course_info(course_id);
        let assignments_fut = async {
            if let Some(assignment_id) = assignment_id {
                let assignment: AssignmentSummary = self
                    .api_get(
                        &format!("/api/v1/courses/{course_id}/assignments/{assignment_id}"),
                        &[("include[]", "submission".into())],
                    )
                    .await?;
                Ok::<_, anyhow::Error>(vec![assignment])
            } else {
                self.assignments(course_id).await
            }
        };
        let (course_res, assignments_res) = tokio::join!(course_fut, assignments_fut);
        let course = course_res?;
        let assignments = assignments_res?;

        let extract_grade = |enrollments: &[CourseEnrollment]| -> Option<TotalGrade> {
            for enrollment in enrollments {
                if enrollment.current_score().is_some()
                    || enrollment.current_grade().is_some()
                    || enrollment.final_score().is_some()
                    || enrollment.final_grade().is_some()
                {
                    return Some(TotalGrade {
                        current_score: enrollment.current_score(),
                        current_grade: enrollment.current_grade().map(str::to_owned),
                        final_score: enrollment.final_score(),
                        final_grade: enrollment.final_grade().map(str::to_owned),
                        current_points: enrollment.grades.as_ref().and_then(|g| g.current_points),
                        html_url: enrollment.grades.as_ref().and_then(|g| g.html_url.clone()),
                    });
                }
            }
            None
        };

        let mut total_grade = extract_grade(&course.enrollments);
        if total_grade.is_none()
            && let Ok(enrollments) = self.course_enrollments(course_id).await
        {
            total_grade = extract_grade(&enrollments);
        }

        let mut assignment_grades = Vec::new();
        for assignment in assignments {
            if let Some(target_id) = assignment_id
                && assignment.id != target_id
            {
                continue;
            }

            let (score, grade, submitted_at, graded_at, late, missing, excused, status) =
                if let Some(sub) = assignment.submission {
                    let status = sub.workflow_state.unwrap_or_else(|| "unsubmitted".into());
                    (
                        sub.score,
                        sub.grade,
                        sub.submitted_at,
                        sub.graded_at,
                        sub.late.unwrap_or(false),
                        sub.missing.unwrap_or(false),
                        sub.excused.unwrap_or(false),
                        status,
                    )
                } else {
                    (
                        None,
                        None,
                        None,
                        None,
                        false,
                        false,
                        false,
                        "unsubmitted".into(),
                    )
                };

            assignment_grades.push(AssignmentGrade {
                assignment_id: assignment.id,
                name: assignment.name,
                points_possible: assignment.points_possible,
                due_at: assignment.due_at,
                grading_type: assignment.grading_type,
                status,
                score,
                grade,
                submitted_at,
                graded_at,
                late,
                missing,
                excused,
            });
        }

        Ok(CourseGradesReport {
            course_id: course.id,
            course_name: course.name,
            total_grade,
            assignments: assignment_grades,
        })
    }

    pub async fn grade_summary(&self) -> anyhow::Result<Vec<CourseGradeSummary>> {
        let courses = self.course_list().await?;
        let mut summaries = Vec::new();

        for course in courses {
            let mut current_score = None;
            let mut current_grade = None;
            let mut final_score = None;
            let mut final_grade = None;
            let mut enrollment_state = None;

            for enrollment in &course.enrollments {
                if enrollment.current_score().is_some() || enrollment.current_grade().is_some() {
                    current_score = enrollment.current_score();
                    current_grade = enrollment.current_grade().map(str::to_owned);
                    final_score = enrollment.final_score();
                    final_grade = enrollment.final_grade().map(str::to_owned);
                    enrollment_state = enrollment.enrollment_state.clone();
                    break;
                }
            }

            if enrollment_state.is_none() {
                enrollment_state = course
                    .enrollments
                    .first()
                    .and_then(|e| e.enrollment_state.clone());
            }

            summaries.push(CourseGradeSummary {
                course_id: course.id,
                course_name: course.name.unwrap_or_else(|| "Untitled Course".into()),
                course_code: course.course_code,
                current_score,
                current_grade,
                final_score,
                final_grade,
                enrollment_state,
            });
        }

        Ok(summaries)
    }
}

#[derive(Deserialize)]
#[serde(untagged)]
enum CanvasId {
    Number(u64),
    Text(String),
}

impl CanvasId {
    fn into_string(self) -> String {
        match self {
            Self::Number(value) => value.to_string(),
            Self::Text(value) => value,
        }
    }
}

fn id<'de, D: serde::Deserializer<'de>>(deserializer: D) -> Result<String, D::Error> {
    Ok(CanvasId::deserialize(deserializer)?.into_string())
}

fn optional_id<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<String>, D::Error> {
    Ok(Option::<CanvasId>::deserialize(deserializer)?.map(CanvasId::into_string))
}

#[derive(Deserialize)]
#[serde(untagged)]
enum FlexibleFloat {
    Number(f64),
    Text(String),
}

fn optional_f64<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<f64>, D::Error> {
    let opt = Option::<FlexibleFloat>::deserialize(deserializer)?;
    match opt {
        Some(FlexibleFloat::Number(n)) => Ok(Some(n)),
        Some(FlexibleFloat::Text(s)) => {
            let trimmed = s.trim();
            if trimmed.is_empty() {
                Ok(None)
            } else {
                trimmed
                    .parse::<f64>()
                    .map(Some)
                    .map_err(serde::de::Error::custom)
            }
        }
        None => Ok(None),
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct PlannerNote {
    #[serde(deserialize_with = "id")]
    pub id: String,
    pub title: Option<String>,
    #[serde(default, alias = "details")]
    pub description: Option<String>,
    #[serde(default, deserialize_with = "optional_id")]
    pub user_id: Option<String>,
    pub workflow_state: Option<String>,
    #[serde(default, deserialize_with = "optional_id")]
    pub course_id: Option<String>,
    pub todo_date: Option<String>,
    pub linked_object_type: Option<String>,
    #[serde(default, deserialize_with = "optional_id")]
    pub linked_object_id: Option<String>,
    pub linked_object_html_url: Option<String>,
    pub linked_object_url: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreatePlannerNotePayload {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub details: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub todo_date: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub course_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub linked_object_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub linked_object_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct UpdatePlannerNotePayload {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub details: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub todo_date: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub course_id: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize, Default)]
pub struct EnrollmentGrades {
    pub html_url: Option<String>,
    #[serde(default, deserialize_with = "optional_f64")]
    pub current_score: Option<f64>,
    pub current_grade: Option<String>,
    #[serde(default, deserialize_with = "optional_f64")]
    pub final_score: Option<f64>,
    pub final_grade: Option<String>,
    #[serde(default, deserialize_with = "optional_f64")]
    pub current_points: Option<f64>,
    #[serde(default, deserialize_with = "optional_f64")]
    pub unposted_current_score: Option<f64>,
    pub unposted_current_grade: Option<String>,
    #[serde(default, deserialize_with = "optional_f64")]
    pub unposted_final_score: Option<f64>,
    pub unposted_final_grade: Option<String>,
    #[serde(default, deserialize_with = "optional_f64")]
    pub unposted_current_points: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TotalGrade {
    pub current_score: Option<f64>,
    pub current_grade: Option<String>,
    pub final_score: Option<f64>,
    pub final_grade: Option<String>,
    pub current_points: Option<f64>,
    pub html_url: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AssignmentGrade {
    pub assignment_id: String,
    pub name: String,
    pub points_possible: Option<f64>,
    pub due_at: Option<String>,
    pub grading_type: Option<String>,
    pub status: String,
    pub score: Option<f64>,
    pub grade: Option<String>,
    pub submitted_at: Option<String>,
    pub graded_at: Option<String>,
    pub late: bool,
    pub missing: bool,
    pub excused: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CourseGradesReport {
    pub course_id: String,
    pub course_name: String,
    pub total_grade: Option<TotalGrade>,
    pub assignments: Vec<AssignmentGrade>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CourseGradeSummary {
    pub course_id: String,
    pub course_name: String,
    pub course_code: Option<String>,
    pub current_score: Option<f64>,
    pub current_grade: Option<String>,
    pub final_score: Option<f64>,
    pub final_grade: Option<String>,
    pub enrollment_state: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct AuthenticatedUser {
    #[serde(deserialize_with = "id")]
    id: String,
    name: String,
    short_name: Option<String>,
    sortable_name: Option<String>,
    login_id: Option<String>,
    avatar_url: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Module {
    #[serde(deserialize_with = "id")]
    id: String,
    name: String,
    position: Option<u64>,
    unlock_at: Option<String>,
    require_sequential_progress: Option<bool>,
    published: Option<bool>,
    state: Option<String>,
    completed_at: Option<String>,
    items_count: u64,
    #[serde(default)]
    items: Vec<ModuleItem>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ModuleItem {
    #[serde(deserialize_with = "id")]
    id: String,
    #[serde(deserialize_with = "id")]
    module_id: String,
    position: Option<u64>,
    title: String,
    #[serde(rename(deserialize = "type"))]
    item_type: String,
    #[serde(default, deserialize_with = "optional_id")]
    content_id: Option<String>,
    indent: Option<u64>,
    html_url: Option<String>,
    page_url: Option<String>,
    external_url: Option<String>,
    new_tab: Option<bool>,
    completion_requirement: Option<CompletionRequirement>,
    content_details: Option<ModuleItemContentDetails>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct CompletionRequirement {
    #[serde(rename(deserialize = "type"))]
    requirement_type: Option<String>,
    min_score: Option<f64>,
    completed: Option<bool>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ModuleItemContentDetails {
    points_possible: Option<f64>,
    due_at: Option<String>,
    unlock_at: Option<String>,
    lock_at: Option<String>,
    locked_for_user: Option<bool>,
    lock_explanation: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct CourseSummary {
    #[serde(deserialize_with = "id")]
    pub id: String,
    pub name: Option<String>,
    pub course_code: Option<String>,
    pub workflow_state: Option<String>,
    pub start_at: Option<String>,
    pub end_at: Option<String>,
    pub time_zone: Option<String>,
    pub term: Option<CourseTerm>,
    #[serde(default)]
    pub teachers: Vec<CourseTeacher>,
    #[serde(default)]
    pub enrollments: Vec<CourseEnrollment>,
    pub is_favorite: Option<bool>,
    pub concluded: Option<bool>,
    pub access_restricted_by_date: Option<bool>,
    #[serde(default, skip_deserializing)]
    pub course_url: String,
    #[serde(default, skip_deserializing)]
    pub modules_url: String,
}

impl CourseSummary {
    fn prepare_urls(&mut self, canvas_origin: &reqwest::Url) {
        self.course_url = format!("{}courses/{}", canvas_origin, self.id);
        self.modules_url = format!("{}/modules", self.course_url);
    }
}

#[derive(Debug, Clone)]
pub struct AttachmentContents {
    pub bytes: Vec<u8>,
    pub mime_type: Option<String>,
    pub filename: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DownloadedFile {
    pub saved_path: String,
    pub filename: String,
    pub bytes: u64,
    pub mime_type: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct AssignmentInfo {
    #[serde(deserialize_with = "id")]
    pub id: String,
    #[serde(deserialize_with = "id")]
    pub course_id: String,
    pub name: String,
    #[serde(rename(deserialize = "description"))]
    pub description_html: Option<String>,
    pub due_at: Option<String>,
    pub unlock_at: Option<String>,
    pub lock_at: Option<String>,
    pub points_possible: Option<f64>,
    pub grading_type: Option<String>,
    #[serde(default)]
    pub submission_types: Vec<String>,
    #[serde(default)]
    pub allowed_extensions: Vec<String>,
    pub html_url: Option<String>,
    pub published: Option<bool>,
    pub locked_for_user: Option<bool>,
    pub lock_explanation: Option<String>,
    #[serde(default)]
    pub attachments: Vec<Attachment>,
    pub submission: Option<AssignmentSubmission>,
}

impl AssignmentInfo {
    fn prepare_resources(&mut self, canvas_origin: &reqwest::Url) -> anyhow::Result<()> {
        self.attachments
            .iter_mut()
            .try_for_each(|attachment| attachment.prepare_resources(canvas_origin))
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct AssignmentSummary {
    #[serde(deserialize_with = "id")]
    pub id: String,
    #[serde(deserialize_with = "id")]
    pub course_id: String,
    pub name: String,
    pub due_at: Option<String>,
    pub unlock_at: Option<String>,
    pub lock_at: Option<String>,
    pub points_possible: Option<f64>,
    pub grading_type: Option<String>,
    #[serde(default)]
    pub submission_types: Vec<String>,
    pub html_url: Option<String>,
    pub published: Option<bool>,
    pub locked_for_user: Option<bool>,
    pub submission: Option<AssignmentSubmission>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct PageInfo {
    #[serde(
        rename(deserialize = "page_id"),
        default,
        deserialize_with = "optional_id"
    )]
    id: Option<String>,
    url: String,
    title: String,
    #[serde(rename(deserialize = "body"))]
    body_html: Option<String>,
    created_at: Option<String>,
    updated_at: Option<String>,
    published: Option<bool>,
    front_page: Option<bool>,
    locked_for_user: Option<bool>,
    lock_explanation: Option<String>,
    html_url: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct FileInfo {
    #[serde(deserialize_with = "id")]
    id: String,
    uuid: Option<String>,
    #[serde(default, deserialize_with = "optional_id")]
    folder_id: Option<String>,
    display_name: Option<String>,
    filename: String,
    #[serde(rename(deserialize = "content-type"), alias = "content_type")]
    content_type: Option<String>,
    size: Option<u64>,
    created_at: Option<String>,
    updated_at: Option<String>,
    modified_at: Option<String>,
    unlock_at: Option<String>,
    lock_at: Option<String>,
    locked: Option<bool>,
    hidden: Option<bool>,
    locked_for_user: Option<bool>,
    lock_explanation: Option<String>,
    mime_class: Option<String>,
    thumbnail_url: Option<String>,
    #[serde(skip_serializing)]
    url: String,
    #[serde(default, skip_deserializing)]
    resource: String,
    #[serde(default, skip_deserializing)]
    download_resource: String,
}

impl FileInfo {
    fn prepare_resources(&mut self, canvas_origin: &reqwest::Url) -> anyhow::Result<()> {
        let path = canvas_resource_path(canvas_origin, &self.url)?;
        self.resource = format!("canvas-text://{path}");
        self.download_resource = format!("canvas://{path}");
        Ok(())
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct CourseInfo {
    #[serde(deserialize_with = "id")]
    pub id: String,
    pub name: String,
    pub course_code: Option<String>,
    #[serde(default, deserialize_with = "optional_id")]
    pub account_id: Option<String>,
    #[serde(default, deserialize_with = "optional_id")]
    pub enrollment_term_id: Option<String>,
    pub start_at: Option<String>,
    pub end_at: Option<String>,
    pub time_zone: Option<String>,
    pub workflow_state: Option<String>,
    pub default_view: Option<String>,
    #[serde(rename(deserialize = "syllabus_body"))]
    pub syllabus_body_html: Option<String>,
    pub term: Option<CourseTerm>,
    #[serde(default)]
    pub teachers: Vec<CourseTeacher>,
    #[serde(default)]
    pub enrollments: Vec<CourseEnrollment>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct CourseTerm {
    #[serde(default, deserialize_with = "optional_id")]
    pub id: Option<String>,
    pub name: Option<String>,
    pub start_at: Option<String>,
    pub end_at: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct CourseTeacher {
    #[serde(default, deserialize_with = "optional_id")]
    pub id: Option<String>,
    pub display_name: Option<String>,
    pub avatar_image_url: Option<String>,
    pub html_url: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct CourseEnrollment {
    #[serde(rename(deserialize = "type"))]
    pub enrollment_type: Option<String>,
    pub role: Option<String>,
    pub enrollment_state: Option<String>,
    #[serde(default)]
    pub grades: Option<EnrollmentGrades>,
    #[serde(default, deserialize_with = "optional_f64")]
    pub computed_current_score: Option<f64>,
    pub computed_current_grade: Option<String>,
    #[serde(default, deserialize_with = "optional_f64")]
    pub computed_final_score: Option<f64>,
    pub computed_final_grade: Option<String>,
}

impl CourseEnrollment {
    pub fn current_score(&self) -> Option<f64> {
        self.grades
            .as_ref()
            .and_then(|g| g.current_score)
            .or(self.computed_current_score)
    }

    pub fn current_grade(&self) -> Option<&str> {
        self.grades
            .as_ref()
            .and_then(|g| g.current_grade.as_deref())
            .or(self.computed_current_grade.as_deref())
    }

    pub fn final_score(&self) -> Option<f64> {
        self.grades
            .as_ref()
            .and_then(|g| g.final_score)
            .or(self.computed_final_score)
    }

    pub fn final_grade(&self) -> Option<&str> {
        self.grades
            .as_ref()
            .and_then(|g| g.final_grade.as_deref())
            .or(self.computed_final_grade.as_deref())
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Attachment {
    #[serde(default, deserialize_with = "optional_id")]
    pub id: Option<String>,
    pub filename: String,
    pub display_name: Option<String>,
    #[serde(rename(deserialize = "content-type"), alias = "content_type")]
    pub content_type: Option<String>,
    pub size: Option<u64>,
    #[serde(skip_serializing)]
    pub url: String,
    #[serde(default, skip_deserializing)]
    pub resource: String,
    #[serde(default, skip_deserializing)]
    pub download_resource: String,
}

impl Attachment {
    fn prepare_resources(&mut self, canvas_origin: &reqwest::Url) -> anyhow::Result<()> {
        let path = canvas_resource_path(canvas_origin, &self.url)?;
        self.resource = format!("canvas-text://{path}");
        self.download_resource = format!("canvas://{path}");
        Ok(())
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct AssignmentSubmission {
    #[serde(default, deserialize_with = "optional_id")]
    pub id: Option<String>,
    pub attempt: Option<u64>,
    pub submitted_at: Option<String>,
    pub workflow_state: Option<String>,
    #[serde(default, deserialize_with = "optional_f64")]
    pub score: Option<f64>,
    pub grade: Option<String>,
    pub late: Option<bool>,
    pub missing: Option<bool>,
    pub excused: Option<bool>,
    pub graded_at: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct DashboardItem {
    pub course_id: Option<String>,
    pub course: Option<String>,
    pub item_type: String,
    pub item_id: Option<String>,
    pub title: String,
    pub scheduled_at: Option<String>,
    pub due_at: Option<String>,
    pub lock_at: Option<String>,
    pub points_possible: Option<f64>,
    pub html_url: Option<String>,
    pub new_activity: bool,
    pub submitted: Option<bool>,
    pub missing: Option<bool>,
    pub late: Option<bool>,
    pub graded: Option<bool>,
    pub excused: Option<bool>,
    pub has_feedback: Option<bool>,
    pub marked_complete: Option<bool>,
    pub dismissed: Option<bool>,
}

#[derive(Debug, Clone, Default)]
pub struct PlannerItemsQuery {
    pub start_date: Option<String>,
    pub end_date: Option<String>,
    pub course_ids: Vec<String>,
    pub filter: Option<String>,
    pub per_page: Option<usize>,
}

fn planner_params(
    query: &PlannerItemsQuery,
    today: chrono::NaiveDate,
) -> Vec<(&'static str, String)> {
    let mut params = vec![(
        "per_page",
        query.per_page.unwrap_or(100).clamp(1, 100).to_string(),
    )];
    match (&query.start_date, &query.end_date) {
        (None, None) => {
            params.push(("start_date", today.to_string()));
            params.push((
                "end_date",
                today
                    .checked_add_months(Months::new(1))
                    .expect("current date supports adding one month")
                    .to_string(),
            ));
        }
        (start, end) => {
            params.extend(start.iter().map(|date| ("start_date", date.clone())));
            params.extend(end.iter().map(|date| ("end_date", date.clone())));
        }
    }
    params.extend(
        query
            .course_ids
            .iter()
            .map(|id| ("context_codes[]", format!("course_{id}"))),
    );
    params.extend(query.filter.iter().map(|value| ("filter", value.clone())));
    params
}

#[derive(Deserialize)]
struct PlannerItem {
    #[serde(default, deserialize_with = "optional_id")]
    course_id: Option<String>,
    #[serde(default, deserialize_with = "optional_id")]
    plannable_id: Option<String>,
    plannable_type: Option<String>,
    context_name: Option<String>,
    plannable_date: Option<String>,
    html_url: Option<String>,
    #[serde(default)]
    new_activity: bool,
    plannable: Plannable,
    submissions: Option<PlannerSubmissions>,
    planner_override: Option<PlannerOverride>,
}

#[derive(Deserialize)]
struct Plannable {
    title: Option<String>,
    name: Option<String>,
    due_at: Option<String>,
    lock_at: Option<String>,
    points_possible: Option<f64>,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum PlannerSubmissions {
    Status(SubmissionStatus),
    None(bool),
}

#[derive(Deserialize)]
struct SubmissionStatus {
    submitted: Option<bool>,
    missing: Option<bool>,
    late: Option<bool>,
    graded: Option<bool>,
    excused: Option<bool>,
    #[serde(alias = "with_feedback")]
    has_feedback: Option<bool>,
}

#[derive(Deserialize)]
struct PlannerOverride {
    marked_complete: Option<bool>,
    dismissed: Option<bool>,
}

impl From<PlannerItem> for DashboardItem {
    fn from(item: PlannerItem) -> Self {
        let submission = match item.submissions {
            Some(PlannerSubmissions::Status(status)) => Some(status),
            Some(PlannerSubmissions::None(value)) => {
                debug_assert!(!value, "Canvas uses false when no submission exists");
                None
            }
            None => None,
        };
        Self {
            course_id: item.course_id,
            course: item.context_name,
            item_type: item.plannable_type.unwrap_or_else(|| "unknown".into()),
            item_id: item.plannable_id,
            title: item
                .plannable
                .title
                .or(item.plannable.name)
                .unwrap_or_else(|| "Untitled Canvas item".into()),
            scheduled_at: item.plannable_date,
            due_at: item.plannable.due_at,
            lock_at: item.plannable.lock_at,
            points_possible: item.plannable.points_possible,
            html_url: item.html_url,
            new_activity: item.new_activity,
            submitted: submission.as_ref().and_then(|value| value.submitted),
            missing: submission.as_ref().and_then(|value| value.missing),
            late: submission.as_ref().and_then(|value| value.late),
            graded: submission.as_ref().and_then(|value| value.graded),
            excused: submission.as_ref().and_then(|value| value.excused),
            has_feedback: submission.and_then(|value| value.has_feedback),
            marked_complete: item
                .planner_override
                .as_ref()
                .and_then(|value| value.marked_complete),
            dismissed: item.planner_override.and_then(|value| value.dismissed),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(target_os = "linux")]
    mod chrome_recovery {
        use super::*;
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

        async fn driver_server(
            responses: Vec<(&'static str, serde_json::Value)>,
        ) -> (String, tokio::task::JoinHandle<()>) {
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let endpoint = format!("http://{}", listener.local_addr().unwrap());
            let task = tokio::spawn(async move {
                for (expected, body) in responses {
                    let (mut stream, _) = listener.accept().await.unwrap();
                    let mut request = Vec::new();
                    while !request.windows(4).any(|bytes| bytes == b"\r\n\r\n") {
                        assert!(stream.read_buf(&mut request).await.unwrap() > 0);
                        assert!(request.len() < 8192);
                    }
                    assert_eq!(
                        std::str::from_utf8(&request).unwrap().lines().next(),
                        Some(expected)
                    );
                    let body = body.to_string();
                    let response = format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                        body.len()
                    );
                    stream.write_all(response.as_bytes()).await.unwrap();
                }
                assert!(
                    tokio::time::timeout(Duration::from_millis(100), listener.accept())
                        .await
                        .is_err(),
                    "unexpected extra ChromeDriver request"
                );
            });
            (endpoint, task)
        }

        fn session(id: &str, profile: &str, pid: u32) -> serde_json::Value {
            serde_json::json!({
                "id": id,
                "capabilities": {
                    "chrome": {"userDataDir": profile},
                    "goog:processID": pid
                }
            })
        }

        #[tokio::test]
        async fn cancelled_native_session_closes_its_webdriver() {
            let (endpoint, server) = driver_server(vec![
                (
                    "POST /session HTTP/1.1",
                    serde_json::json!({"value":{"sessionId":"native-cancel","capabilities":{}}}),
                ),
                (
                    "POST /session/native-cancel/timeouts HTTP/1.1",
                    serde_json::json!({"value":null}),
                ),
                (
                    "DELETE /session/native-cancel HTTP/1.1",
                    serde_json::json!({"value":null}),
                ),
            ])
            .await;
            let driver = WebDriver::new(&endpoint, DesiredCapabilities::chrome())
                .await
                .unwrap();
            drop(BrowserSession {
                driver: Some(driver),
                profile_lock: None,
            });
            tokio::time::timeout(Duration::from_secs(5), server)
                .await
                .unwrap()
                .unwrap();
        }

        #[tokio::test]
        async fn successful_native_session_quit_is_not_replayed_on_drop() {
            let (endpoint, server) = driver_server(vec![
                (
                    "POST /session HTTP/1.1",
                    serde_json::json!({"value":{"sessionId":"native-quit","capabilities":{}}}),
                ),
                (
                    "POST /session/native-quit/timeouts HTTP/1.1",
                    serde_json::json!({"value":null}),
                ),
                (
                    "DELETE /session/native-quit HTTP/1.1",
                    serde_json::json!({"value":null}),
                ),
            ])
            .await;
            let driver = WebDriver::new(&endpoint, DesiredCapabilities::chrome())
                .await
                .unwrap();
            BrowserSession {
                driver: Some(driver),
                profile_lock: None,
            }
            .quit()
            .await
            .unwrap();
            server.await.unwrap();
        }

        #[tokio::test]
        async fn closes_only_the_matching_profile_and_process() {
            let (endpoint, server) = driver_server(vec![
                (
                    "GET /sessions HTTP/1.1",
                    serde_json::json!({"value": [
                        session("other-profile", "/other-profile", 42),
                        session("other-process", "/canvas-profile", 43),
                        session("canvas-session", "/canvas-profile", 42)
                    ]}),
                ),
                (
                    "DELETE /session/canvas-session HTTP/1.1",
                    serde_json::json!({"value": null}),
                ),
            ])
            .await;
            close_orphaned_chrome_session(&endpoint, Path::new("/canvas-profile"), 42)
                .await
                .unwrap();
            server.await.unwrap();
        }

        #[tokio::test]
        async fn stops_the_orphaned_driver_after_closing_its_only_session() {
            let (endpoint, server) = driver_server(vec![
                (
                    "GET /sessions HTTP/1.1",
                    serde_json::json!({"value": [
                        session("canvas-session", "/canvas-profile", 42)
                    ]}),
                ),
                (
                    "DELETE /session/canvas-session HTTP/1.1",
                    serde_json::json!({"value": null}),
                ),
                ("GET /shutdown HTTP/1.1", serde_json::json!({"value": null})),
            ])
            .await;
            close_orphaned_chrome_session(&endpoint, Path::new("/canvas-profile"), 42)
                .await
                .unwrap();
            server.await.unwrap();
        }

        #[tokio::test]
        async fn refuses_to_close_a_session_without_matching_profile_and_process() {
            let (endpoint, server) = driver_server(vec![(
                "GET /sessions HTTP/1.1",
                serde_json::json!({"value": [
                    session("other-profile", "/other-profile", 42),
                    session("other-process", "/canvas-profile", 43)
                ]}),
            )])
            .await;
            let error = close_orphaned_chrome_session(&endpoint, Path::new("/canvas-profile"), 42)
                .await
                .unwrap_err();
            assert!(
                error
                    .to_string()
                    .contains("no matching orphaned Canvas session")
            );
            server.await.unwrap();
        }

        #[tokio::test]
        async fn does_not_shutdown_driver_when_session_cleanup_fails() {
            let (endpoint, server) = driver_server(vec![
                (
                    "GET /sessions HTTP/1.1",
                    serde_json::json!({"value": [
                        session("canvas-session", "/canvas-profile", 42)
                    ]}),
                ),
                (
                    "DELETE /session/canvas-session HTTP/1.1",
                    serde_json::json!({
                        "value": {"error": "unknown error", "message": "could not quit"}
                    }),
                ),
            ])
            .await;
            assert!(
                close_orphaned_chrome_session(&endpoint, Path::new("/canvas-profile"), 42)
                    .await
                    .is_err()
            );
            server.await.unwrap();
        }

        #[test]
        fn does_not_treat_the_current_process_as_an_orphaned_browser() {
            assert!(
                orphaned_chromedriver(Path::new("/canvas-profile"), std::process::id()).is_none()
            );
            assert!(!process_is_user_systemd(std::process::id()));
        }

        #[tokio::test]
        async fn reports_a_live_profile_owner_without_removing_its_lock() {
            let profile = std::env::temp_dir()
                .join(format!("canvas_profile_owner_test_{}", std::process::id()));
            std::fs::create_dir(&profile).unwrap();
            let hostname = std::fs::read_to_string("/proc/sys/kernel/hostname").unwrap();
            let owner = format!("{}-{}", hostname.trim(), std::process::id());
            let lock = profile.join("SingletonLock");
            std::os::unix::fs::symlink(&owner, &lock).unwrap();
            let error = recover_orphaned_chrome(&profile).await.unwrap_err();
            assert!(error.to_string().contains("is in use by process"));
            assert_eq!(std::fs::read_link(&lock).unwrap(), Path::new(&owner));
            std::fs::remove_file(lock).unwrap();
            std::fs::remove_dir(profile).unwrap();
        }
    }

    fn example_origin() -> reqwest::Url {
        reqwest::Url::parse("https://canvas.example.edu").unwrap()
    }
    #[tokio::test]
    async fn course_front_page_uses_canvas_front_page_endpoint() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let origin =
            reqwest::Url::parse(&format!("http://{}", listener.local_addr().unwrap())).unwrap();
        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut bytes = Vec::new();
            while !bytes.windows(4).any(|b| b == b"\r\n\r\n") {
                assert!(stream.read_buf(&mut bytes).await.unwrap() > 0);
            }
            let request = String::from_utf8(bytes).unwrap();
            assert_eq!(
                request
                    .split_whitespace()
                    .nth(1)
                    .unwrap()
                    .trim_end_matches('?'),
                "/api/v1/courses/9/front_page"
            );
            let body=serde_json::json!({"page_id":7,"url":"welcome","title":"Welcome","body":"<h1>Course home</h1>","front_page":true}).to_string();
            stream.write_all(format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",body.len()).as_bytes()).await.unwrap();
        });
        let api = CanvasApi::with_origin(origin).unwrap();
        *api.cookie_header.write().await = "session=local-test-only".into();
        let page = tokio::time::timeout(Duration::from_secs(5), api.course_front_page("9"))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(page.title, "Welcome");
        assert_eq!(page.body_html.as_deref(), Some("<h1>Course home</h1>"));
        assert_eq!(page.front_page, Some(true));
        server.await.unwrap();
        assert!(api.course_front_page("not-an-id").await.is_err());
    }
    #[tokio::test]
    async fn attachment_get_reuses_a_concurrently_refreshed_cookie_once() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let origin =
            reqwest::Url::parse(&format!("http://{}", listener.local_addr().unwrap())).unwrap();
        let api = CanvasApi::with_origin(origin).unwrap();
        *api.cookie_header.write().await = "session=old-local-test".into();
        let shared = api.clone();
        let server = tokio::spawn(async move {
            for attempt in 0..2 {
                let (mut stream, _) = listener.accept().await.unwrap();
                let mut bytes = Vec::new();
                while !bytes.windows(4).any(|b| b == b"\r\n\r\n") {
                    assert!(stream.read_buf(&mut bytes).await.unwrap() > 0);
                }
                let request = String::from_utf8(bytes).unwrap();
                if attempt == 0 {
                    assert!(request.contains("session=old-local-test"));
                    *shared.cookie_header.write().await = "session=new-local-test".into();
                    stream.write_all(b"HTTP/1.1 401 Unauthorized\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").await.unwrap();
                } else {
                    assert!(request.contains("session=new-local-test"));
                    stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok").await.unwrap();
                }
            }
        });
        assert_eq!(
            api.attachment("/files/1/download").await.unwrap().bytes,
            b"ok"
        );
        server.await.unwrap();
    }

    #[tokio::test]
    async fn missing_and_partial_module_items_load_in_parallel_and_keep_order() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        async fn head(stream: &mut tokio::net::TcpStream) -> String {
            let mut bytes = Vec::new();
            while !bytes.windows(4).any(|b| b == b"\r\n\r\n") {
                assert!(stream.read_buf(&mut bytes).await.unwrap() > 0);
                assert!(bytes.len() < 16384);
            }
            String::from_utf8(bytes).unwrap()
        }
        async fn respond(stream: &mut tokio::net::TcpStream, value: serde_json::Value) {
            let body = value.to_string();
            stream.write_all(format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",body.len()).as_bytes()).await.unwrap();
        }
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let origin =
            reqwest::Url::parse(&format!("http://{}", listener.local_addr().unwrap())).unwrap();
        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let request = head(&mut stream).await;
            assert!(request.starts_with("GET /api/v1/courses/9/modules?"));
            respond(&mut stream,serde_json::json!([
                {"id":"10","name":"Partial","items_count":2,"items":[{"id":"old","module_id":"10","title":"Partial embedded value","type":"Page"}]},
                {"id":"20","name":"Missing","items_count":1,"items":[]},
                {"id":"30","name":"Complete","items_count":1,"items":[{"id":"31","module_id":"30","title":"Already complete","type":"Page"}]}
            ])).await;
            drop(stream);
            // Both GETs must arrive before either response is released. A
            // sequential implementation times out here instead of passing.
            let (mut a, _) = tokio::time::timeout(Duration::from_secs(3), listener.accept())
                .await
                .unwrap()
                .unwrap();
            let ah = head(&mut a).await;
            let (mut b, _) = tokio::time::timeout(Duration::from_secs(3), listener.accept())
                .await
                .unwrap()
                .unwrap();
            let bh = head(&mut b).await;
            for (stream, request) in [(&mut b, bh), (&mut a, ah)] {
                if request.contains("/modules/10/items?") {
                    respond(stream,serde_json::json!([{"id":"11","module_id":"10","title":"First","type":"Page"},{"id":"12","module_id":"10","title":"Second","type":"Page"}])).await;
                } else {
                    assert!(request.contains("/modules/20/items?"));
                    respond(stream,serde_json::json!([{"id":"21","module_id":"20","title":"Other","type":"Page"}])).await;
                }
            }
        });
        let api = CanvasApi::with_origin(origin).unwrap();
        *api.cookie_header.write().await = "session=local-test-only".into();
        let modules = tokio::time::timeout(Duration::from_secs(5), api.modules("9"))
            .await
            .unwrap()
            .unwrap();
        server.await.unwrap();
        assert_eq!(
            modules.iter().map(|m| m.id.as_str()).collect::<Vec<_>>(),
            ["10", "20", "30"]
        );
        assert_eq!(modules[0].items.len(), 2);
        assert_eq!(modules[0].items[0].title, "First");
        assert_eq!(modules[1].items[0].title, "Other");
        assert_eq!(modules[2].items[0].title, "Already complete");
    }

    #[tokio::test]
    async fn preview_download_limit_checks_chunked_bodies() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let origin =
            reqwest::Url::parse(&format!("http://{}", listener.local_addr().unwrap())).unwrap();
        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut request = Vec::new();
            while !request.windows(4).any(|b| b == b"\r\n\r\n") {
                stream.read_buf(&mut request).await.unwrap();
            }
            stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: application/pdf\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\nb\r\nhello world\r\n0\r\n\r\n").await.unwrap();
        });
        let api = CanvasApi::with_origin(origin).unwrap();
        *api.cookie_header.write().await = "session=local-test-only".into();
        let error = api
            .attachment_limited("/files/1/download", 10)
            .await
            .unwrap_err();
        assert!(error.to_string().contains("exceeds"));
        server.await.unwrap();
    }
    #[tokio::test]
    async fn one_assignment_grade_does_not_fetch_the_entire_assignment_list() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let origin =
            reqwest::Url::parse(&format!("http://{}", listener.local_addr().unwrap())).unwrap();
        let server = tokio::spawn(async move {
            for _ in 0..2 {
                let (mut stream, _) = listener.accept().await.unwrap();
                let mut request = Vec::new();
                while !request.windows(4).any(|b| b == b"\r\n\r\n") {
                    assert!(stream.read_buf(&mut request).await.unwrap() > 0);
                }
                let request = String::from_utf8(request).unwrap();
                let body=if request.starts_with("GET /api/v1/courses/9/assignments/7?"){
                    serde_json::json!({"id":"7","course_id":"9","name":"Selected assignment","points_possible":20,"submission":{"workflow_state":"graded","score":19,"grade":"19"}})
                }else{assert!(request.starts_with("GET /api/v1/courses/9?"));serde_json::json!({"id":"9","name":"Course","enrollments":[{"computed_current_score":90,"computed_current_grade":"A"}]})}.to_string();
                stream.write_all(format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",body.len()).as_bytes()).await.unwrap();
            }
        });
        let api = CanvasApi::with_origin(origin).unwrap();
        *api.cookie_header.write().await = "session=local-test-only".into();
        let report =
            tokio::time::timeout(Duration::from_secs(5), api.course_grades("9", Some("7")))
                .await
                .unwrap()
                .unwrap();
        server.await.unwrap();
        assert_eq!(report.assignments.len(), 1);
        assert_eq!(report.assignments[0].assignment_id, "7");
        assert_eq!(report.assignments[0].score, Some(19.));
    }

    #[test]
    fn validates_configured_canvas_origins() {
        assert_eq!(
            parse_canvas_url("https://canvas.example.edu").unwrap(),
            example_origin()
        );
        assert!(parse_canvas_url("ftp://canvas.example.edu").is_err());
        assert!(parse_canvas_url("https://canvas.example.edu/courses").is_err());
    }

    #[tokio::test]
    async fn requests_use_cached_cookies_shared_across_clones() {
        let api = CanvasApi::with_origin(example_origin()).unwrap();
        let clone = api.clone();
        assert!(clone.cookie_header.read().await.is_empty());

        *api.cookie_header.write().await = "session=first".into();
        assert_eq!(clone.canvas_cookie_header().await.unwrap(), "session=first");

        *api.cookie_header.write().await = "session=refreshed".into();
        assert_eq!(
            clone.canvas_cookie_header().await.unwrap(),
            "session=refreshed"
        );
    }

    #[tokio::test]
    async fn replacing_cached_cookies_rejects_an_empty_browser_session() {
        let api = CanvasApi::with_origin(example_origin()).unwrap();
        api.replace_cached_cookie_header("session=current".into())
            .await
            .unwrap();

        assert!(
            api.replace_cached_cookie_header(String::new())
                .await
                .is_err()
        );
        assert_eq!(api.canvas_cookie_header().await.unwrap(), "session=current");
    }

    #[tokio::test]
    async fn concurrent_initial_cookie_loads_run_once() {
        use std::sync::atomic::{AtomicUsize, Ordering};

        let api = CanvasApi::with_origin(example_origin()).unwrap();
        let loads = Arc::new(AtomicUsize::new(0));
        let mut requests = tokio::task::JoinSet::new();
        for _ in 0..20 {
            let api = api.clone();
            let loads = loads.clone();
            requests.spawn(async move {
                api.replace_cookie_header_if_current(Some(""), async {
                    loads.fetch_add(1, Ordering::SeqCst);
                    tokio::time::sleep(Duration::from_millis(20)).await;
                    Ok("session=loaded".into())
                })
                .await
                .unwrap()
            });
        }
        while let Some(result) = requests.join_next().await {
            assert_eq!(result.unwrap(), "session=loaded");
        }
        assert_eq!(loads.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn concurrent_unauthorized_requests_share_a_cookie_refresh() {
        use std::sync::atomic::{AtomicUsize, Ordering};

        let api = CanvasApi::with_origin(example_origin()).unwrap();
        api.replace_cached_cookie_header("session=stale".into())
            .await
            .unwrap();
        let loads = Arc::new(AtomicUsize::new(0));
        let mut requests = tokio::task::JoinSet::new();
        for _ in 0..20 {
            let api = api.clone();
            let loads = loads.clone();
            requests.spawn(async move {
                api.replace_cookie_header_if_current(Some("session=stale"), async {
                    loads.fetch_add(1, Ordering::SeqCst);
                    tokio::time::sleep(Duration::from_millis(20)).await;
                    Ok("session=refreshed".into())
                })
                .await
                .unwrap()
            });
        }
        while let Some(result) = requests.join_next().await {
            assert_eq!(result.unwrap(), "session=refreshed");
        }
        assert_eq!(loads.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn failed_cookie_load_can_be_retried_without_restarting() {
        let api = CanvasApi::with_origin(example_origin()).unwrap();
        assert!(
            api.replace_cookie_header_if_current(Some(""), async {
                anyhow::bail!("browser unavailable")
            })
            .await
            .is_err()
        );
        assert!(api.cookie_header.read().await.is_empty());

        assert_eq!(
            api.replace_cookie_header_if_current(Some(""), async {
                Ok("session=recovered".into())
            })
            .await
            .unwrap(),
            "session=recovered"
        );
    }

    #[tokio::test]
    async fn failed_refresh_preserves_existing_cookies_and_allows_recovery() {
        let api = CanvasApi::with_origin(example_origin()).unwrap();
        api.replace_cached_cookie_header("session=stale".into())
            .await
            .unwrap();
        assert!(
            api.replace_cookie_header_if_current(Some("session=stale"), async {
                Ok(String::new())
            })
            .await
            .is_err()
        );
        assert_eq!(api.canvas_cookie_header().await.unwrap(), "session=stale");
        assert_eq!(
            api.replace_cookie_header_if_current(Some("session=stale"), async {
                Ok("session=recovered".into())
            })
            .await
            .unwrap(),
            "session=recovered"
        );
    }

    #[tokio::test]
    async fn cancelled_cookie_load_does_not_hold_the_reload_lock() {
        let api = CanvasApi::with_origin(example_origin()).unwrap();
        let started = Arc::new(tokio::sync::Notify::new());
        let task = tokio::spawn({
            let api = api.clone();
            let started = started.clone();
            async move {
                api.replace_cookie_header_if_current(Some(""), async {
                    started.notify_one();
                    std::future::pending::<anyhow::Result<String>>().await
                })
                .await
            }
        });
        started.notified().await;
        task.abort();
        assert!(task.await.unwrap_err().is_cancelled());
        let recovered = tokio::time::timeout(
            Duration::from_secs(1),
            api.replace_cookie_header_if_current(Some(""), async {
                Ok("session=recovered".into())
            }),
        )
        .await
        .unwrap()
        .unwrap();
        assert_eq!(recovered, "session=recovered");
    }

    #[tokio::test]
    async fn explicit_cookie_refresh_runs_even_with_cached_cookies() {
        let api = CanvasApi::with_origin(example_origin()).unwrap();
        api.replace_cached_cookie_header("session=old".into())
            .await
            .unwrap();
        assert_eq!(
            api.replace_cookie_header_if_current(None, async { Ok("session=new".into()) })
                .await
                .unwrap(),
            "session=new"
        );
    }

    #[test]
    fn retries_only_safe_get_requests_after_unauthorized() {
        assert!(should_refresh_and_retry(
            &reqwest::Method::GET,
            reqwest::StatusCode::UNAUTHORIZED
        ));
        assert!(!should_refresh_and_retry(
            &reqwest::Method::POST,
            reqwest::StatusCode::UNAUTHORIZED
        ));
        assert!(!should_refresh_and_retry(
            &reqwest::Method::GET,
            reqwest::StatusCode::INTERNAL_SERVER_ERROR
        ));
    }

    #[test]
    fn attachment_resources_preserve_canvas_download_urls() {
        for prefix in ["canvas://", "canvas-text://"] {
            let url = attachment_url(
                &example_origin(),
                &format!("{prefix}files/99/download?download_frd=1&verifier=example"),
            )
            .unwrap();
            assert_eq!(
                url.as_str(),
                "https://canvas.example.edu/files/99/download?download_frd=1&verifier=example"
            );
        }
    }

    #[test]
    fn attachment_resources_reject_foreign_origins_before_using_cookies() {
        for prefix in ["canvas://", "canvas-text://"] {
            for payload in [
                "https://other.example/collect",
                "https://canvas.example.edu.other.example/collect",
                "https://canvas.example.edu@other.example/collect",
                "http://canvas.example.edu/files/99",
                "https://canvas.example.edu:444/files/99",
                "//other.example/collect",
                " https://other.example/collect",
                "\thttps://other.example/collect",
                "https:\\other.example\\collect",
                "file:///etc/passwd",
                "https://user:password@canvas.example.edu/files/99",
                "",
            ] {
                assert!(
                    attachment_url(&example_origin(), &format!("{prefix}{payload}")).is_err(),
                    "accepted unsafe resource payload: {payload:?}"
                );
            }
        }
    }

    #[test]
    fn parses_next_pagination_link() {
        let header = reqwest::header::HeaderValue::from_static(
            r#"<https://canvas.example.edu/api/v1/planner/items?page=1>; rel="current", <https://canvas.example.edu/api/v1/planner/items?page=2>; rel="next""#,
        );
        assert_eq!(
            next_link(Some(&header)).unwrap().as_deref(),
            Some("https://canvas.example.edu/api/v1/planner/items?page=2")
        );
    }

    #[test]
    fn planner_defaults_to_one_calendar_month() {
        let params = planner_params(
            &PlannerItemsQuery::default(),
            chrono::NaiveDate::from_ymd_opt(2026, 1, 31).unwrap(),
        );

        assert!(params.contains(&("start_date", "2026-01-31".into())));
        assert!(params.contains(&("end_date", "2026-02-28".into())));
    }

    #[test]
    fn planner_preserves_an_explicit_partial_range() {
        let params = planner_params(
            &PlannerItemsQuery {
                start_date: Some("2026-09-01".into()),
                ..Default::default()
            },
            chrono::NaiveDate::from_ymd_opt(2026, 1, 1).unwrap(),
        );

        assert!(params.contains(&("start_date", "2026-09-01".into())));
        assert!(!params.iter().any(|(key, _)| *key == "end_date"));
    }

    #[test]
    fn deserializes_numeric_and_string_ids() {
        let numeric: ModuleItem = serde_json::from_value(serde_json::json!({
            "id": 10, "module_id": "20", "title": "Syllabus",
            "type": "File", "content_id": 30
        }))
        .unwrap();

        assert_eq!(numeric.id, "10");
        assert_eq!(numeric.module_id, "20");
        assert_eq!(numeric.content_id.as_deref(), Some("30"));

        let output = serde_json::to_value(numeric).unwrap();
        assert_eq!(output["item_type"], "File");
        assert!(output.get("type").is_none());
    }

    #[test]
    fn preserves_canvas_field_names() {
        let page: PageInfo = serde_json::from_value(serde_json::json!({
            "page_id": 5,
            "url": "syllabus",
            "title": "Syllabus",
            "body": "<p>Textbook</p>"
        }))
        .unwrap();
        let output = serde_json::to_value(page).unwrap();

        assert_eq!(output["id"], "5");
        assert_eq!(output["body_html"], "<p>Textbook</p>");
        assert!(output.get("page_id").is_none());
        assert!(output.get("body").is_none());
    }

    #[test]
    fn flattens_planner_items() {
        let raw: PlannerItem = serde_json::from_value(serde_json::json!({
            "course_id": "42",
            "plannable_id": "99",
            "plannable_type": "assignment",
            "context_name": "Example Course",
            "new_activity": true,
            "submissions": {"submitted": false, "missing": true},
            "planner_override": {"marked_complete": false},
            "plannable": {
                "title": "Example Assignment",
                "due_at": "2026-09-12T03:59:59Z",
                "points_possible": 100
            }
        }))
        .unwrap();
        let item: DashboardItem = raw.into();

        assert_eq!(item.course_id.as_deref(), Some("42"));
        assert_eq!(item.title, "Example Assignment");
        assert_eq!(item.missing, Some(true));
        assert_eq!(item.marked_complete, Some(false));
    }

    #[test]
    fn prepares_course_and_file_links() {
        let mut course: CourseSummary = serde_json::from_value(serde_json::json!({
            "id": 42,
            "name": "Example Course",
            "access_restricted_by_date": false
        }))
        .unwrap();
        course.prepare_urls(&example_origin());

        let mut file: FileInfo = serde_json::from_value(serde_json::json!({
            "id": 99,
            "filename": "example.pdf",
            "content-type": "application/pdf",
            "url": "https://canvas.example.edu/files/99/download?download_frd=1"
        }))
        .unwrap();
        file.prepare_resources(&example_origin()).unwrap();

        assert_eq!(
            course.modules_url,
            "https://canvas.example.edu/courses/42/modules"
        );
        assert_eq!(
            file.resource,
            "canvas-text://files/99/download?download_frd=1"
        );
    }

    #[test]
    fn rejects_off_origin_file_links() {
        let mut file: FileInfo = serde_json::from_value(serde_json::json!({
            "id": 1,
            "filename": "unsafe.pdf",
            "url": "https://example.com/unsafe.pdf"
        }))
        .unwrap();

        assert!(file.prepare_resources(&example_origin()).is_err());
    }

    #[test]
    fn encodes_page_slugs() {
        assert_eq!(
            canvas_api_path(
                &example_origin(),
                &["courses", "42", "pages", "week 1/readings"]
            )
            .unwrap(),
            "/api/v1/courses/42/pages/week%201%2Freadings"
        );
    }

    #[test]
    fn extracts_and_decodes_csrf_tokens() {
        let cookie = "session=123; _csrf_token=abc%2Bdef%3D%2Fghi; other=xyz";
        assert_eq!(extract_csrf_token(cookie).as_deref(), Some("abc+def=/ghi"));

        let unencoded = "_csrf_token=simple_token_123; session=456";
        assert_eq!(
            extract_csrf_token(unencoded).as_deref(),
            Some("simple_token_123")
        );

        let no_csrf = "session=123; user_id=456";
        assert_eq!(extract_csrf_token(no_csrf), None);
    }

    #[test]
    fn deserializes_planner_notes_with_flexible_ids_and_aliases() {
        let json = serde_json::json!({
            "id": 234,
            "title": "Bring books tomorrow",
            "details": "I need to bring books tomorrow for my course on biology",
            "user_id": 1578941,
            "workflow_state": "active",
            "course_id": 1578941,
            "todo_date": "2017-05-09T10:12:00Z",
            "linked_object_type": "assignment",
            "linked_object_id": 131072,
            "linked_object_html_url": "https://canvas.example.com/courses/1578941/assignments/131072",
            "linked_object_url": "https://canvas.example.com/api/v1/courses/1578941/assignments/131072"
        });

        let note: PlannerNote = serde_json::from_value(json).unwrap();
        assert_eq!(note.id, "234");
        assert_eq!(note.title.as_deref(), Some("Bring books tomorrow"));
        assert_eq!(
            note.description.as_deref(),
            Some("I need to bring books tomorrow for my course on biology")
        );
        assert_eq!(note.user_id.as_deref(), Some("1578941"));
        assert_eq!(note.course_id.as_deref(), Some("1578941"));
        assert_eq!(note.linked_object_type.as_deref(), Some("assignment"));
        assert_eq!(note.linked_object_id.as_deref(), Some("131072"));
    }

    #[test]
    fn serializes_planner_note_payloads() {
        let payload = CreatePlannerNotePayload {
            title: Some("Study for Exam".into()),
            details: Some("Chapters 1-4".into()),
            todo_date: Some("2026-09-20".into()),
            course_id: Some("42".into()),
            linked_object_type: None,
            linked_object_id: None,
        };

        let val = serde_json::to_value(&payload).unwrap();
        assert_eq!(val["title"], "Study for Exam");
        assert_eq!(val["details"], "Chapters 1-4");
        assert_eq!(val["todo_date"], "2026-09-20");
        assert_eq!(val["course_id"], "42");
        assert!(val.get("linked_object_type").is_none());

        let update_payload = UpdatePlannerNotePayload {
            title: Some("Updated Title".into()),
            details: None,
            todo_date: None,
            course_id: Some("".into()),
        };
        let update_val = serde_json::to_value(&update_payload).unwrap();
        assert_eq!(update_val["title"], "Updated Title");
        assert_eq!(update_val["course_id"], "");
        assert!(update_val.get("details").is_none());
    }

    #[test]
    fn deserializes_course_enrollment_with_flat_and_nested_grades() {
        // Test flat computed_* fields
        let flat_json = serde_json::json!({
            "type": "StudentEnrollment",
            "role": "StudentEnrollment",
            "enrollment_state": "active",
            "computed_current_score": 92.5,
            "computed_current_grade": "A",
            "computed_final_score": 88.0,
            "computed_final_grade": "B+"
        });
        let flat_enrollment: CourseEnrollment = serde_json::from_value(flat_json).unwrap();
        assert_eq!(flat_enrollment.current_score(), Some(92.5));
        assert_eq!(flat_enrollment.current_grade(), Some("A"));
        assert_eq!(flat_enrollment.final_score(), Some(88.0));
        assert_eq!(flat_enrollment.final_grade(), Some("B+"));

        // Test nested grades hash with string-formatted scores and empty string
        let nested_json = serde_json::json!({
            "type": "StudentEnrollment",
            "role": "StudentEnrollment",
            "enrollment_state": "active",
            "grades": {
                "html_url": "https://canvas.example.edu/courses/42/grades",
                "current_score": "95.5",
                "current_grade": "A",
                "final_score": "",
                "final_grade": null,
                "current_points": 190.5
            }
        });
        let nested_enrollment: CourseEnrollment = serde_json::from_value(nested_json).unwrap();
        assert_eq!(nested_enrollment.current_score(), Some(95.5));
        assert_eq!(nested_enrollment.current_grade(), Some("A"));
        assert_eq!(nested_enrollment.final_score(), None);
        assert_eq!(nested_enrollment.final_grade(), None);
        assert_eq!(
            nested_enrollment
                .grades
                .as_ref()
                .and_then(|g| g.current_points),
            Some(190.5)
        );
    }

    #[test]
    fn extracts_content_disposition_filenames() {
        assert_eq!(
            extract_content_disposition_filename(r#"attachment; filename="assignment 1.pdf""#),
            Some("assignment 1.pdf".to_owned())
        );
        assert_eq!(
            extract_content_disposition_filename("attachment; filename=data.csv"),
            Some("data.csv".to_owned())
        );
        assert_eq!(
            extract_content_disposition_filename(r#"inline; filename="notes.docx""#),
            Some("notes.docx".to_owned())
        );
        assert_eq!(
            extract_content_disposition_filename(
                "attachment; filename*=UTF-8''my%20test%20file.pdf"
            ),
            Some("my test file.pdf".to_owned())
        );
        assert_eq!(
            extract_content_disposition_filename(r#"attachment; FILENAME="capital.zip""#),
            Some("capital.zip".to_owned())
        );
        assert_eq!(
            parse_content_disposition_params(
                r#"attachment; filename="my;problem;set.pdf"; size=100"#
            ),
            vec![
                ("filename".to_owned(), "my;problem;set.pdf".to_owned()),
                ("size".to_owned(), "100".to_owned()),
            ]
        );
        assert_eq!(
            extract_content_disposition_filename(
                r#"attachment; filename="my;problem;set.pdf"; size=100"#
            ),
            Some("my_problem_set.pdf".to_owned())
        );
        assert_eq!(
            extract_content_disposition_filename(
                r#"attachment; filename="quotes \"escaped\".pdf""#
            ),
            Some("quotes _escaped_.pdf".to_owned())
        );
        assert_eq!(extract_content_disposition_filename("attachment"), None);
    }

    #[test]
    fn sanitizes_download_filenames_against_path_traversal() {
        assert_eq!(sanitize_filename("../../etc/passwd"), "passwd");
        assert_eq!(
            sanitize_filename("..\\..\\Windows\\System32\\cmd.exe"),
            "cmd.exe"
        );
        assert_eq!(sanitize_filename("/var/log/test.txt"), "test.txt");
        assert_eq!(
            sanitize_filename("C:\\Users\\victim\\Desktop\\hack.exe"),
            "hack.exe"
        );
        assert_eq!(sanitize_filename("\\\\server\\share\\hack.exe"), "hack.exe");
        assert_eq!(sanitize_filename("foo:bar*baz?.txt"), "foo_bar_baz_.txt");
        assert_eq!(sanitize_filename("   "), "attachment");
        assert_eq!(sanitize_filename("..."), "attachment");
        assert_eq!(sanitize_filename(".."), "attachment");
        assert_eq!(sanitize_filename("."), "attachment");
        assert_eq!(sanitize_filename("test.pdf...   "), "test.pdf");
        assert_eq!(
            sanitize_filename("%2e%2e%2f%2e%2e%2fsecret.txt"),
            "secret.txt"
        );
        assert_eq!(sanitize_filename("test\u{202e}fdp.exe"), "test_fdp.exe");
        assert_eq!(
            sanitize_filename("test\r\n\t\0file.txt"),
            "test____file.txt"
        );
    }

    #[test]
    fn sanitizes_windows_reserved_device_names() {
        assert_eq!(sanitize_filename("CON"), "_CON");
        assert_eq!(sanitize_filename("con.txt"), "_con.txt");
        assert_eq!(sanitize_filename("AUX.pdf"), "_AUX.pdf");
        assert_eq!(sanitize_filename("nul.zip"), "_nul.zip");
        assert_eq!(sanitize_filename("com1.tar.gz"), "_com1.tar.gz");
        assert_eq!(sanitize_filename("lpt9"), "_lpt9");
        assert_eq!(sanitize_filename("contact.txt"), "contact.txt");
    }

    #[test]
    fn truncates_overly_long_filenames_preserving_extension() {
        let long_name = format!("{}.pdf", "a".repeat(300));
        let sanitized = sanitize_filename(&long_name);
        assert!(sanitized.len() <= 200);
        assert!(sanitized.ends_with(".pdf"));
    }

    #[test]
    fn attachment_url_accepts_relative_and_full_urls() {
        let origin = example_origin();
        assert_eq!(
            attachment_url(&origin, "canvas://files/10/download")
                .unwrap()
                .as_str(),
            "https://canvas.example.edu/files/10/download"
        );
        assert_eq!(
            attachment_url(&origin, "canvas-text://files/10/download")
                .unwrap()
                .as_str(),
            "https://canvas.example.edu/files/10/download"
        );
        assert_eq!(
            attachment_url(&origin, "https://canvas.example.edu/files/10/download")
                .unwrap()
                .as_str(),
            "https://canvas.example.edu/files/10/download"
        );
        assert_eq!(
            attachment_url(&origin, "/files/10/download")
                .unwrap()
                .as_str(),
            "https://canvas.example.edu/files/10/download"
        );
        assert_eq!(
            attachment_url(&origin, "files/10/download")
                .unwrap()
                .as_str(),
            "https://canvas.example.edu/files/10/download"
        );
    }

    #[test]
    fn canvas_browser_links_must_stay_on_the_configured_origin() {
        let origin = "https://canvas.example.edu";
        assert_eq!(
            canvas_browser_target(origin, "/courses/4/modules/items/5")
                .unwrap()
                .as_str(),
            "https://canvas.example.edu/courses/4/modules/items/5"
        );
        assert_eq!(
            canvas_browser_target(
                origin,
                "https://canvas.example.edu/courses/4/modules/items/5"
            )
            .unwrap()
            .as_str(),
            "https://canvas.example.edu/courses/4/modules/items/5"
        );
        assert!(!is_canvas_url(origin, "https://lti.tool.test/launch"));
        assert!(!is_canvas_url(
            origin,
            "https://other.example.edu/courses/4"
        ));
        assert!(!is_canvas_url(origin, "javascript:alert(1)"));
        assert!(!is_canvas_url(
            origin,
            "https://user:secret@canvas.example.edu/courses/4"
        ));
    }

    #[test]
    fn extracts_file_id_and_generates_safe_attachment_filename() {
        assert_eq!(
            extract_file_id_from_resource("canvas://files/12345/download"),
            Some("12345".to_string())
        );
        assert_eq!(
            extract_file_id_from_resource("canvas-text://files/987/download?verifier=abc"),
            Some("987".to_string())
        );
        assert_eq!(
            extract_file_id_from_resource("https://canvas.example.edu/files/555/download"),
            Some("555".to_string())
        );

        assert_eq!(
            safe_attachment_filename("canvas://files/12345/download", "homework.pdf"),
            "12345_homework.pdf"
        );
        assert_eq!(
            safe_attachment_filename("canvas://files/12345/download", "12345_homework.pdf"),
            "12345_homework.pdf"
        );
        assert_eq!(
            safe_attachment_filename("canvas://files/12345/download", "12345-homework.pdf"),
            "12345-homework.pdf"
        );
        assert_eq!(
            safe_attachment_filename("canvas://files/12345/download", "CON.pdf"),
            "12345__CON.pdf"
        );
    }

    #[tokio::test]
    async fn resolves_and_contains_paths_safely() {
        let expected_dir = default_download_dir();
        let default_path = resolve_and_contain_path(None, "123_test.pdf")
            .await
            .unwrap();
        assert_eq!(default_path, expected_dir.join("123_test.pdf"));

        let custom_dir = resolve_and_contain_path(Some("downloads/subdir/"), "123_test.pdf")
            .await
            .unwrap();
        assert!(custom_dir.ends_with(std::path::Path::new("subdir").join("123_test.pdf")));

        let custom_file = resolve_and_contain_path(Some("downloads/custom.pdf"), "123_test.pdf")
            .await
            .unwrap();
        assert!(custom_file.ends_with(std::path::Path::new("downloads").join("custom.pdf")));
    }

    #[tokio::test]
    async fn saves_attachment_to_disk_and_returns_metadata() {
        let api = CanvasApi::with_origin(example_origin()).unwrap();

        let temp_dir = std::env::temp_dir().join(format!(
            "canvas_test_{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let dest = temp_dir.to_string_lossy().into_owned();

        let bytes = b"test content for auto download";
        let downloaded = api
            .save_attachment_to_disk(
                "canvas://files/7890/download",
                bytes,
                Some("lab-report.pdf"),
                Some("application/pdf"),
                Some(&dest),
            )
            .await
            .unwrap();

        assert_eq!(downloaded.filename, "7890_lab-report.pdf");
        assert_eq!(downloaded.bytes, bytes.len() as u64);
        assert_eq!(downloaded.mime_type.as_deref(), Some("application/pdf"));
        assert!(std::path::Path::new(&downloaded.saved_path).exists());

        let read_back = tokio::fs::read(&downloaded.saved_path).await.unwrap();
        assert_eq!(read_back, bytes);

        let _ = tokio::fs::remove_dir_all(temp_dir).await;
    }
}
