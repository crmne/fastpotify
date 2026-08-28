//! Official pinned yt-dlp: extract locally, prefer a strictly newer user binary.
//!
//! No runtime download. No cookies, credentials, or self-update flags.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

include!(concat!(env!("OUT_DIR"), "/bundled_ytdlp.rs"));

const VERSION_TIMEOUT: Duration = Duration::from_secs(3);
const VERSION_STDOUT_LIMIT: usize = 4096;
const SIDECAR_NAME: &str = "yt-dlp.bundle.json";
static UNIQUE_COUNTER: AtomicU64 = AtomicU64::new(0);

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct YtDlpVersion {
    year: u16,
    month: u8,
    day: u8,
    extra: Vec<u32>,
}

impl std::fmt::Display for YtDlpVersion {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:04}.{:02}.{:02}", self.year, self.month, self.day)?;
        for part in &self.extra {
            write!(f, ".{part}")?;
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum YtDlpOrigin {
    Bundled,
    User,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResolvedYtDlp {
    pub path: PathBuf,
    pub version: Option<YtDlpVersion>,
    pub origin: YtDlpOrigin,
}

#[derive(Clone, Debug)]
pub(crate) struct BundledRef {
    pub path: PathBuf,
    pub version: YtDlpVersion,
}

#[derive(Clone, Debug)]
pub(crate) struct UserProbe {
    pub path: PathBuf,
    pub version: Option<YtDlpVersion>,
}

#[derive(Clone, Debug)]
pub struct BundlePayload<'a> {
    pub bytes: &'a [u8],
    pub version: &'a str,
    pub sha256: &'a str,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
struct Sidecar {
    version: String,
    sha256: String,
    length: u64,
    #[serde(default)]
    file: Option<String>,
}

pub fn has_bundled_ytdlp() -> bool {
    BUNDLED_YTDLP_BYTES.is_some()
}

pub fn bundled_payload() -> Option<BundlePayload<'static>> {
    Some(BundlePayload {
        bytes: BUNDLED_YTDLP_BYTES?,
        version: BUNDLED_YTDLP_VERSION?,
        sha256: BUNDLED_YTDLP_SHA256?,
    })
}

pub fn user_ytdlp_present(configured: Option<&str>) -> bool {
    if let Some(path) = configured.filter(|path| !path.is_empty())
        && Path::new(path).is_file()
    {
        return true;
    }
    first_path_candidate().is_some()
}

pub fn resolve(configured: Option<&str>, extract_dir: &Path) -> Option<ResolvedYtDlp> {
    let users = probe_user_candidates(configured);
    let bundled = install_bundle_if_needed(extract_dir, &users);
    select_ytdlp(bundled.as_ref(), &users)
}

fn install_bundle_if_needed(extract_dir: &Path, users: &[UserProbe]) -> Option<BundledRef> {
    let payload = bundled_payload()?;
    let version = parse_ytdlp_version(payload.version)?;
    let user_wins = users.iter().any(|user| {
        user.version
            .as_ref()
            .is_some_and(|candidate| *candidate > version)
    });
    if user_wins {
        return None;
    }
    match extract_bundle(extract_dir, &payload) {
        Ok(path) => Some(BundledRef { path, version }),
        Err(error) => {
            log::warn!("yt-dlp extract failed: {error}");
            None
        }
    }
}

pub fn log_choice(resolved: &ResolvedYtDlp) {
    let source = match resolved.origin {
        YtDlpOrigin::Bundled => "bundled",
        YtDlpOrigin::User => "user",
    };
    let version = resolved
        .version
        .as_ref()
        .map(|version| version.to_string())
        .unwrap_or_else(|| "unknown".into());
    log::info!(
        "yt-dlp source={source} version={version} path={}",
        resolved.path.display()
    );
}

pub fn extract_bundle(dest_dir: &Path, payload: &BundlePayload<'_>) -> Result<PathBuf, String> {
    let expected = payload.sha256.trim().to_ascii_lowercase();
    let got = sha256_hex(payload.bytes);
    if got != expected {
        return Err(format!(
            "yt-dlp bundle bytes do not match the pinned hash ({got})"
        ));
    }
    fs::create_dir_all(dest_dir)
        .map_err(|error| format!("unable to create yt-dlp dir: {error}"))?;
    let dest = dest_dir.join(bundled_bin_name());
    let sidecar_path = dest_dir.join(SIDECAR_NAME);
    let length = payload.bytes.len() as u64;
    if let Some(existing) = installed_bundle_path(dest_dir, payload.version, &expected, length) {
        ensure_unix_executable(&existing);
        return Ok(existing);
    }
    let tmp = dest_dir.join(format!(
        "{}.tmp.{}-{}",
        bundled_bin_name(),
        std::process::id(),
        unique_suffix()
    ));
    write_atomic_bytes(&tmp, payload.bytes)?;
    let written =
        fs::read(&tmp).map_err(|error| format!("unable to re-read yt-dlp temp: {error}"))?;
    let written_hash = sha256_hex(&written);
    if written_hash != expected {
        let _ = fs::remove_file(&tmp);
        return Err("yt-dlp temp file failed hash verification".into());
    }
    let final_path = replace_or_fallback(&tmp, &dest, &expected)?;
    if let Err(error) = write_sidecar(
        &sidecar_path,
        &Sidecar {
            version: payload.version.to_string(),
            sha256: expected,
            length,
            file: final_path
                .file_name()
                .map(|name| name.to_string_lossy().into_owned()),
        },
    ) {
        log::warn!("yt-dlp sidecar write failed: {error}");
    }
    ensure_unix_executable(&final_path);
    Ok(final_path)
}

pub(crate) fn select_ytdlp(
    bundled: Option<&BundledRef>,
    users: &[UserProbe],
) -> Option<ResolvedYtDlp> {
    let best_parseable = users
        .iter()
        .filter_map(|user| {
            user.version
                .clone()
                .map(|version| (version, user.path.clone()))
        })
        .max_by(|left, right| left.0.cmp(&right.0));

    if let Some(bundled) = bundled {
        if let Some((version, path)) = best_parseable
            && version > bundled.version
        {
            return Some(ResolvedYtDlp {
                path,
                version: Some(version),
                origin: YtDlpOrigin::User,
            });
        }
        return Some(ResolvedYtDlp {
            path: bundled.path.clone(),
            version: Some(bundled.version.clone()),
            origin: YtDlpOrigin::Bundled,
        });
    }

    if let Some((version, path)) = best_parseable {
        return Some(ResolvedYtDlp {
            path,
            version: Some(version),
            origin: YtDlpOrigin::User,
        });
    }

    users.first().map(|user| ResolvedYtDlp {
        path: user.path.clone(),
        version: None,
        origin: YtDlpOrigin::User,
    })
}

pub fn parse_ytdlp_version(raw: &str) -> Option<YtDlpVersion> {
    for word in
        raw.split(|ch: char| ch.is_whitespace() || matches!(ch, '[' | ']' | '(' | ')' | ',' | ';'))
    {
        let word = word.trim_matches(|ch: char| !ch.is_ascii_digit() && ch != '.');
        if let Some(version) = parse_dotted(word) {
            return Some(version);
        }
    }
    None
}

#[cfg(test)]
pub(crate) fn embed_target(os: &str, arch: &str) -> Option<&'static str> {
    match (os, arch) {
        ("windows", "x86_64") => Some("x86_64-pc-windows-msvc"),
        ("windows", "aarch64") => Some("aarch64-pc-windows-msvc"),
        ("linux", "x86_64") => Some("x86_64-unknown-linux-gnu"),
        ("linux", "aarch64") => Some("aarch64-unknown-linux-gnu"),
        ("macos", "aarch64") => Some("aarch64-apple-darwin"),
        ("macos", "x86_64") => Some("x86_64-apple-darwin"),
        _ => None,
    }
}

fn parse_dotted(word: &str) -> Option<YtDlpVersion> {
    let parts: Vec<&str> = word.split('.').collect();
    if parts.len() < 3 {
        return None;
    }
    let year: u16 = parts[0].parse().ok()?;
    let month: u8 = parts[1].parse().ok()?;
    let day: u8 = parts[2].parse().ok()?;
    if !(2010..=2100).contains(&year) || !(1..=12).contains(&month) || !(1..=31).contains(&day) {
        return None;
    }
    let mut extra = Vec::new();
    for part in &parts[3..] {
        extra.push(part.parse().ok()?);
    }
    Some(YtDlpVersion {
        year,
        month,
        day,
        extra,
    })
}

fn probe_user_candidates(configured: Option<&str>) -> Vec<UserProbe> {
    collect_user_candidates(configured)
        .into_iter()
        .map(|path| UserProbe {
            version: probe_version(&path),
            path,
        })
        .collect()
}

fn collect_user_candidates(configured: Option<&str>) -> Vec<PathBuf> {
    let mut out = Vec::new();
    if let Some(path) = configured.filter(|path| !path.is_empty()) {
        let path = PathBuf::from(path);
        if path.is_file() {
            out.push(path);
        }
    }
    for candidate in path_candidates() {
        if out.iter().any(|existing| existing == &candidate) {
            continue;
        }
        out.push(candidate);
    }
    out
}

fn first_path_candidate() -> Option<PathBuf> {
    path_candidates().into_iter().next()
}

fn path_candidates() -> Vec<PathBuf> {
    let names: &[&str] = if cfg!(windows) {
        &["yt-dlp.exe", "yt-dlp"]
    } else {
        &["yt-dlp"]
    };
    let path = std::env::var_os("PATH").unwrap_or_default();
    let mut out = Vec::new();
    for dir in std::env::split_paths(&path) {
        for name in names {
            let candidate = dir.join(name);
            if candidate.is_file() {
                out.push(candidate);
            }
        }
    }
    out
}

fn probe_version(path: &Path) -> Option<YtDlpVersion> {
    parse_ytdlp_version(&run_version(path)?)
}

fn run_version(path: &Path) -> Option<String> {
    let mut command = std::process::Command::new(path);
    command
        .arg("--version")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        command.creation_flags(CREATE_NO_WINDOW);
    }
    let mut child = command.spawn().ok()?;
    let mut stdout = child.stdout.take()?;
    let reader = std::thread::spawn(move || {
        let mut buf = Vec::new();
        let mut chunk = [0u8; 256];
        loop {
            match stdout.read(&mut chunk) {
                Ok(0) => break,
                Ok(n) => {
                    if buf.len() + n > VERSION_STDOUT_LIMIT {
                        break;
                    }
                    buf.extend_from_slice(&chunk[..n]);
                }
                Err(_) => break,
            }
        }
        buf
    });
    let deadline = Instant::now() + VERSION_TIMEOUT;
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                let buf = reader.join().ok()?;
                if !status.success() {
                    return None;
                }
                return String::from_utf8(buf).ok();
            }
            Ok(None) if Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(20));
            }
            Ok(None) => {
                let _ = child.kill();
                let _ = child.wait();
                let _ = reader.join();
                return None;
            }
            Err(_) => return None,
        }
    }
}

fn bundled_bin_name() -> &'static str {
    if cfg!(windows) {
        "yt-dlp.exe"
    } else {
        "yt-dlp"
    }
}

fn fallback_bin_name() -> &'static str {
    if cfg!(windows) {
        "yt-dlp.fastpotify.exe"
    } else {
        "yt-dlp.fastpotify"
    }
}

fn installed_bundle_path(
    dest_dir: &Path,
    version: &str,
    sha256: &str,
    length: u64,
) -> Option<PathBuf> {
    let text = fs::read_to_string(dest_dir.join(SIDECAR_NAME)).ok()?;
    let sidecar: Sidecar = serde_json::from_str(&text).ok()?;
    if sidecar.version != version
        || sidecar.sha256.to_ascii_lowercase() != sha256
        || sidecar.length != length
    {
        return None;
    }
    let mut candidates = Vec::new();
    if let Some(name) = sidecar.file.as_deref().and_then(safe_sidecar_name) {
        candidates.push(dest_dir.join(name));
    }
    for name in [bundled_bin_name(), fallback_bin_name()] {
        let path = dest_dir.join(name);
        if !candidates.iter().any(|existing| existing == &path) {
            candidates.push(path);
        }
    }
    candidates
        .into_iter()
        .find(|path| file_matches(path, sha256, length))
}

fn safe_sidecar_name(name: &str) -> Option<&str> {
    let name = name.trim();
    if name.is_empty() {
        return None;
    }
    let mut parts = Path::new(name).components();
    match (parts.next(), parts.next()) {
        (Some(std::path::Component::Normal(_)), None) => Some(name),
        _ => None,
    }
}

fn file_matches(path: &Path, sha256: &str, length: u64) -> bool {
    let Ok(meta) = fs::metadata(path) else {
        return false;
    };
    if meta.len() != length {
        return false;
    }
    fs::read(path)
        .ok()
        .is_some_and(|bytes| sha256_hex(&bytes) == sha256)
}

fn write_atomic_bytes(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let mut file = OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .open(path)
        .map_err(|error| format!("unable to create yt-dlp temp: {error}"))?;
    file.write_all(bytes)
        .map_err(|error| format!("unable to write yt-dlp temp: {error}"))?;
    let _ = file.sync_all();
    Ok(())
}

fn write_sidecar(path: &Path, sidecar: &Sidecar) -> Result<(), String> {
    let text = serde_json::to_string(sidecar)
        .map_err(|error| format!("unable to encode yt-dlp sidecar: {error}"))?;
    let tmp = path.with_extension("json.tmp");
    {
        let mut file = File::create(&tmp)
            .map_err(|error| format!("unable to create yt-dlp sidecar: {error}"))?;
        file.write_all(text.as_bytes())
            .map_err(|error| format!("unable to write yt-dlp sidecar: {error}"))?;
        let _ = file.sync_all();
    }
    if path.exists() {
        let _ = fs::remove_file(path);
    }
    fs::rename(&tmp, path).map_err(|error| {
        let _ = fs::remove_file(&tmp);
        format!("unable to install yt-dlp sidecar: {error}")
    })
}

fn replace_or_fallback(tmp: &Path, dest: &Path, expected_hash: &str) -> Result<PathBuf, String> {
    if dest.exists() {
        #[cfg(windows)]
        {
            match fs::remove_file(dest) {
                Ok(()) => {}
                Err(error) if is_in_use(&error) => {
                    if file_matches(dest, expected_hash, file_len(dest)) {
                        let _ = fs::remove_file(tmp);
                        return Ok(dest.to_path_buf());
                    }
                    let fallback = dest.with_file_name(fallback_bin_name());
                    match fs::rename(tmp, &fallback) {
                        Ok(()) => return Ok(fallback),
                        Err(rename_error) if is_in_use(&rename_error) => {
                            if file_matches(&fallback, expected_hash, file_len(&fallback)) {
                                let _ = fs::remove_file(tmp);
                                return Ok(fallback);
                            }
                            return Err(format!(
                                "yt-dlp is in use and could not be replaced: {rename_error}"
                            ));
                        }
                        Err(rename_error) => {
                            let _ = fs::remove_file(tmp);
                            return Err(format!("unable to place yt-dlp fallback: {rename_error}"));
                        }
                    }
                }
                Err(error) => {
                    let _ = fs::remove_file(tmp);
                    return Err(format!("unable to replace yt-dlp: {error}"));
                }
            }
        }
    }
    match fs::rename(tmp, dest) {
        Ok(()) => Ok(dest.to_path_buf()),
        Err(error) => {
            let _ = fs::remove_file(tmp);
            Err(format!("unable to install yt-dlp: {error}"))
        }
    }
}

#[cfg(windows)]
fn file_len(path: &Path) -> u64 {
    fs::metadata(path).map(|meta| meta.len()).unwrap_or(0)
}

#[cfg(windows)]
fn is_in_use(error: &std::io::Error) -> bool {
    matches!(error.raw_os_error(), Some(5) | Some(32) | Some(33))
}

fn ensure_unix_executable(path: &Path) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if let Ok(metadata) = fs::metadata(path) {
            let mut perms = metadata.permissions();
            perms.set_mode(0o755);
            let _ = fs::set_permissions(path, perms);
        }
    }
    #[cfg(not(unix))]
    {
        let _ = path;
    }
}

fn unique_suffix() -> String {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or(0);
    let sequence = UNIQUE_COUNTER.fetch_add(1, Ordering::Relaxed);
    format!("{nanos}-{sequence}")
}

fn sha256_hex(bytes: &[u8]) -> String {
    hex_lower(&Sha256::digest(bytes))
}

fn hex_lower(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push(HEX[(byte >> 4) as usize] as char);
        out.push(HEX[(byte & 0x0f) as usize] as char);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ver(raw: &str) -> YtDlpVersion {
        parse_ytdlp_version(raw).unwrap_or_else(|| panic!("parse {raw}"))
    }

    fn scratch_dir() -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "fastpotify-ytdlp-{}-{}",
            std::process::id(),
            unique_suffix()
        ));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn version_parse_stable_nightly_garbage() {
        assert_eq!(ver("yt-dlp 2026.08.19").to_string(), "2026.08.19");
        assert_eq!(ver("2026.08.19").to_string(), "2026.08.19");
        assert_eq!(
            ver("yt-dlp 2026.08.19.232845").to_string(),
            "2026.08.19.232845"
        );
        assert_eq!(ver("yt-dlp 2026.8.19 [abc123]").to_string(), "2026.08.19");
        assert!(parse_ytdlp_version("not a version").is_none());
        assert!(parse_ytdlp_version("1.2.3").is_none());
        assert!(parse_ytdlp_version("").is_none());
        assert!(parse_ytdlp_version("yt-dlp unknown").is_none());
        assert!(ver("2026.08.19.232845") > ver("2026.08.19"));
        assert!(ver("2026.08.20") > ver("2026.08.19.999999"));
        assert_eq!(ver("2026.08.19"), ver("yt-dlp 2026.08.19"));
    }

    #[test]
    fn select_prefers_strictly_newer_user() {
        let bundled = BundledRef {
            path: PathBuf::from("bundled"),
            version: ver("2026.08.19"),
        };
        let newer = UserProbe {
            path: PathBuf::from("user-new"),
            version: Some(ver("2026.08.19.1")),
        };
        let equal = UserProbe {
            path: PathBuf::from("user-eq"),
            version: Some(ver("2026.08.19")),
        };
        let older = UserProbe {
            path: PathBuf::from("user-old"),
            version: Some(ver("2025.01.01")),
        };
        let junk = UserProbe {
            path: PathBuf::from("user-junk"),
            version: None,
        };

        let picked = select_ytdlp(Some(&bundled), std::slice::from_ref(&newer)).unwrap();
        assert_eq!(picked.origin, YtDlpOrigin::User);
        assert_eq!(picked.path, PathBuf::from("user-new"));

        for users in [
            vec![equal.clone()],
            vec![older.clone()],
            vec![junk.clone()],
            vec![equal.clone(), older.clone(), junk.clone()],
            Vec::new(),
        ] {
            let picked = select_ytdlp(Some(&bundled), &users).unwrap();
            assert_eq!(picked.origin, YtDlpOrigin::Bundled);
            assert_eq!(picked.path, PathBuf::from("bundled"));
        }

        let mixed = select_ytdlp(Some(&bundled), &[older, junk, newer, equal]).unwrap();
        assert_eq!(mixed.origin, YtDlpOrigin::User);
        assert_eq!(mixed.path, PathBuf::from("user-new"));
    }

    #[test]
    fn select_without_bundle_preserves_user() {
        let parseable = UserProbe {
            path: PathBuf::from("a"),
            version: Some(ver("2024.01.01")),
        };
        let newer = UserProbe {
            path: PathBuf::from("b"),
            version: Some(ver("2025.01.01")),
        };
        let junk = UserProbe {
            path: PathBuf::from("c"),
            version: None,
        };
        let picked = select_ytdlp(None, &[parseable, newer, junk.clone()]).unwrap();
        assert_eq!(picked.path, PathBuf::from("b"));
        assert_eq!(picked.origin, YtDlpOrigin::User);

        let only_junk = select_ytdlp(None, &[junk]).unwrap();
        assert_eq!(only_junk.path, PathBuf::from("c"));
        assert!(only_junk.version.is_none());

        assert!(select_ytdlp(None, &[]).is_none());
    }

    #[test]
    fn failed_extract_falls_back_to_older_or_unparseable_user() {
        let older = UserProbe {
            path: PathBuf::from("user-old"),
            version: Some(ver("2020.01.01")),
        };
        let junk = UserProbe {
            path: PathBuf::from("user-junk"),
            version: None,
        };
        let picked = select_ytdlp(None, std::slice::from_ref(&older)).unwrap();
        assert_eq!(picked.origin, YtDlpOrigin::User);
        assert_eq!(picked.path, PathBuf::from("user-old"));
        let picked = select_ytdlp(None, std::slice::from_ref(&junk)).unwrap();
        assert_eq!(picked.origin, YtDlpOrigin::User);
        assert_eq!(picked.path, PathBuf::from("user-junk"));
        assert!(picked.version.is_none());
    }

    #[test]
    fn missing_configured_path_still_collects_path_candidates() {
        let missing = collect_user_candidates(Some("/no/such/fastpotify-ytdlp"));
        let path_only = collect_user_candidates(None);
        assert_eq!(missing, path_only);
    }

    #[test]
    fn extract_is_idempotent_and_hash_mismatch_keeps_dest() {
        let dir = scratch_dir();
        let good = b"fastpotify-fake-ytdlp";
        let good_hash = sha256_hex(good);
        let payload = BundlePayload {
            bytes: good,
            version: "2026.08.19",
            sha256: &good_hash,
        };
        let dest = extract_bundle(&dir, &payload).unwrap();
        let first = fs::read(&dest).unwrap();
        let dest_again = extract_bundle(&dir, &payload).unwrap();
        assert_eq!(dest, dest_again);
        assert_eq!(fs::read(&dest_again).unwrap(), first);

        let bad = b"tampered-bytes";
        let mismatch = extract_bundle(
            &dir,
            &BundlePayload {
                bytes: bad,
                version: "2026.08.19",
                sha256: &good_hash,
            },
        );
        assert!(mismatch.is_err());
        assert_eq!(fs::read(&dest).unwrap(), first);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn extract_replaces_when_payload_is_new_and_valid() {
        let dir = scratch_dir();
        let first = b"first-bytes";
        let first_hash = sha256_hex(first);
        extract_bundle(
            &dir,
            &BundlePayload {
                bytes: first,
                version: "2026.08.19",
                sha256: &first_hash,
            },
        )
        .unwrap();
        let second = b"second-bytes-ok";
        let second_hash = sha256_hex(second);
        let dest = extract_bundle(
            &dir,
            &BundlePayload {
                bytes: second,
                version: "2026.08.20",
                sha256: &second_hash,
            },
        )
        .unwrap();
        assert_eq!(fs::read(&dest).unwrap(), second);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn extract_trusts_sidecar_fallback_path_without_rewrite() {
        let dir = scratch_dir();
        let good = b"fallback-install-bytes";
        let good_hash = sha256_hex(good);
        let fallback = dir.join(fallback_bin_name());
        fs::write(&fallback, good).unwrap();
        write_sidecar(
            &dir.join(SIDECAR_NAME),
            &Sidecar {
                version: "2026.08.19".into(),
                sha256: good_hash.clone(),
                length: good.len() as u64,
                file: Some(fallback_bin_name().into()),
            },
        )
        .unwrap();
        let dest = dir.join(bundled_bin_name());
        fs::write(&dest, b"stale-locked-dest").unwrap();
        let payload = BundlePayload {
            bytes: good,
            version: "2026.08.19",
            sha256: &good_hash,
        };
        let used = extract_bundle(&dir, &payload).unwrap();
        assert_eq!(used, fallback);
        assert_eq!(fs::read(&dest).unwrap(), b"stale-locked-dest");
        assert_eq!(fs::read(&fallback).unwrap(), good);
        let used_again = extract_bundle(&dir, &payload).unwrap();
        assert_eq!(used_again, fallback);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn bundle_seams_supported_and_missing_targets() {
        assert_eq!(
            embed_target("windows", "x86_64"),
            Some("x86_64-pc-windows-msvc")
        );
        assert_eq!(
            embed_target("windows", "aarch64"),
            Some("aarch64-pc-windows-msvc")
        );
        assert_eq!(
            embed_target("linux", "x86_64"),
            Some("x86_64-unknown-linux-gnu")
        );
        assert_eq!(
            embed_target("linux", "aarch64"),
            Some("aarch64-unknown-linux-gnu")
        );
        assert_eq!(
            embed_target("macos", "aarch64"),
            Some("aarch64-apple-darwin")
        );
        assert_eq!(embed_target("macos", "x86_64"), Some("x86_64-apple-darwin"));
        assert_eq!(embed_target("freebsd", "x86_64"), None);
        assert_eq!(embed_target("linux", "arm"), None);
        match (
            BUNDLED_YTDLP_BYTES,
            BUNDLED_YTDLP_SHA256,
            BUNDLED_YTDLP_VERSION,
        ) {
            (Some(bytes), Some(hash), Some(version)) => {
                assert_eq!(sha256_hex(bytes), hash);
                assert!(parse_ytdlp_version(version).is_some());
                assert!(has_bundled_ytdlp());
                assert!(bytes.len() > 1_000_000);
            }
            (None, None, None) => {
                assert!(!has_bundled_ytdlp());
            }
            other => panic!("inconsistent bundled yt-dlp constants: {other:?}"),
        }
    }

    #[test]
    fn probe_missing_binary_is_none() {
        assert!(probe_version(Path::new("no-such-ytdlp-binary-fastpotify-test")).is_none());
    }
}
