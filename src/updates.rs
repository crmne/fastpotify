//! Daily update check against GitHub releases, and self-update download +
//! install for Linux hand-installs.

use std::path::PathBuf;
use std::time::Duration;

use anyhow::{Context, Result};
use serde::Deserialize;
use sha2::{Digest, Sha256};

const LATEST_RELEASE_URL: &str = "https://api.github.com/repos/crmne/fastpotify/releases/latest";

/// Update-check interval.
pub const CHECK_INTERVAL: Duration = Duration::from_secs(24 * 60 * 60);

/// Build the full asset filename for the current target given a version tag.
/// The release workflow publishes `fastpotify-<tag>-<target>.tar.gz` where
/// `<tag>` already includes its `v` prefix (e.g. `v0.7.1`).
fn archive_name_for_target(tag: &str) -> Option<String> {
    let target = match (std::env::consts::ARCH, std::env::consts::OS) {
        ("x86_64", "linux") => format!("fastpotify-{tag}-x86_64-unknown-linux-gnu.tar.gz"),
        ("aarch64", "linux") => format!("fastpotify-{tag}-aarch64-unknown-linux-gnu.tar.gz"),
        _ => return None,
    };
    Some(target)
}

/// Build the asset download URL for a release tag, when the current target
/// has a published archive.
pub fn download_url(tag: &str) -> Option<String> {
    let name = archive_name_for_target(tag)?;
    Some(format!(
        "https://github.com/crmne/fastpotify/releases/download/{tag}/{name}",
        tag = tag,
        name = name
    ))
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Release {
    /// The version number, without a leading `v`.
    pub version: String,
    /// The release page, with every download.
    pub url: String,
    /// The platform-specific downloadable archive for this build, when one
    /// exists for the current target. Empty when the target is unknown or
    /// when the release does not carry a matching artifact.
    pub download_url: String,
}

#[derive(Deserialize)]
struct LatestRelease {
    tag_name: String,
    html_url: String,
}

/// The newest release, when it is newer than this build.
pub async fn newer_release(http: &reqwest::Client) -> Result<Option<Release>> {
    let latest: LatestRelease = http
        .get(LATEST_RELEASE_URL)
        .header("Accept", "application/vnd.github+json")
        .send()
        .await?
        .error_for_status()?
        .json()
        .await
        .context("unexpected release listing")?;
    let version = latest.tag_name.trim_start_matches('v').to_string();
    let download_url =
        crate::updates::download_url(&latest.tag_name).unwrap_or_default();
    Ok(
        is_newer(&version, env!("CARGO_PKG_VERSION")).then_some(Release {
            version,
            url: latest.html_url,
            download_url,
        }),
    )
}

/// `major.minor.patch`, and whether a `-rc1` or similar suffix marks it
/// as a pre-release of that version; anything else is `None`.
fn parse(version: &str) -> Option<([u64; 3], bool)> {
    let version = version.trim();
    let (numbers, pre_release) = match version.split_once('-') {
        Some((numbers, _)) => (numbers, true),
        None => (version, false),
    };
    let mut parts = numbers.split('.').map(|part| part.parse::<u64>().ok());
    Some((
        [parts.next()??, parts.next()??, parts.next()??],
        pre_release,
    ))
}

/// Whether `candidate` is a newer stable version than `current`.
/// Stable releases supersede their release candidates. Other prereleases and
/// invalid versions are ignored.
pub fn is_newer(candidate: &str, current: &str) -> bool {
    match (parse(candidate), parse(current)) {
        (Some((candidate, false)), Some((current, current_pre))) => {
            candidate > current || (candidate == current && current_pre)
        }
        _ => false,
    }
}

// ---- self-update --------------------------------------------------------

/// Download the newer-release archive for the current platform, verify it
/// against the release's checksums file, extract the new binary over this
/// one, and relaunch. Non-Linux targets and targets without a published
/// archive return an explanatory error.
///
/// The caller is responsible for relaunching the new binary after this
/// returns `Ok(())`; see the app's `launch_updated_binary` for the platform
///-appropriate restart.
pub async fn apply_update(http: &reqwest::Client, dirs: &crate::paths::AppDirs, release: &Release) -> Result<PathBuf> {
    let tag = format!("v{}", release.version);
    let url = if release.download_url.is_empty() {
        return Err(anyhow::anyhow!("no self-update archive for this platform"));
    } else {
        release.download_url.clone()
    };
    let checksums_url = format!("https://github.com/crmne/fastpotify/releases/download/{tag}/checksums.txt");

    // Download the archive and the checksums file in parallel.
    let (archive_bytes, checksums_text) = tokio::join!(
        async { http.get(&url).send().await?.error_for_status()?.bytes().await },
        async { http.get(&checksums_url).send().await?.error_for_status()?.text().await },
    );
    let archive = archive_bytes?;
    let checksums = checksums_text?;
    log::debug!("downloaded archive: {} bytes from {}", archive.len(), url);

    // Find the expected SHA-256 for our archive name.
    let archive_name = url
        .rsplit('/')
        .next()
        .ok_or_else(|| anyhow::anyhow!("bad download url"))?;
    let expected_hash = checksums
        .lines()
        .find_map(|line| {
            let mut parts = line.split_whitespace();
            let hash = parts.next()?;
            let name = parts.next()?;
            if name == archive_name {
                Some(hash.to_string())
            } else {
                None
            }
        })
        .ok_or_else(|| anyhow::anyhow!("no checksum for {}", archive_name))?;

    // Verify.
    let actual = format!("{:x}", Sha256::digest(&archive));
    if !actual.eq_ignore_ascii_case(&expected_hash) {
        anyhow::bail!("checksum mismatch for {}", archive_name);
    }
    log::debug!("checksum verified for {archive_name}");

    // Extract the archive to a temp dir, find the new binary, and replace
    // the running one. We write into the app's cache dir so the user does not
    // need to own the install prefix.
    let tmp = dirs.cache_dir().join("update");
    std::fs::create_dir_all(&tmp)?;

    let archive_path = tmp.join(&archive_name);
    std::fs::write(&archive_path, &archive)?;

    let tar_gz = flate2::read::GzDecoder::new(std::io::Cursor::new(archive));
    let mut archive_fs = tar::Archive::new(tar_gz);
    archive_fs.unpack(&tmp)?;
    log::debug!("extracted archive to {tmp:?}");

    // The archive contains a folder named `fastpotify-<tag>-<target>/` with
    // the binary at the top. Use the tag as-is (with `v`) to match the archive's
    // internal folder naming.
    let prefix = format!("fastpotify-{tag}-");
    log::debug!("looking for prefix: {prefix}");
    if let Ok(entries) = std::fs::read_dir(&tmp) {
        for entry in entries.flatten() {
            log::debug!("tmp entry: {:?}", entry.file_name());
        }
    }
    let unpacked = std::fs::read_dir(&tmp)?
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().map_or(false, |t| t.is_dir()))
        .find(|e| e.file_name().to_string_lossy().starts_with(&prefix))
        .ok_or_else(|| anyhow::anyhow!("no unpacked directory starting with {prefix}"))?;
    let unpacked_dir = unpacked.path();
    log::debug!("unpacked dir: {:?}", unpacked_dir);

    // Locate the binary inside. On Linux it is `fastpotify` (no .exe).
    #[cfg(target_os = "linux")]
    let binary_name = "fastpotify";
    #[cfg(target_os = "macos")]
    let binary_name = "fastpotify";
    #[cfg(target_os = "windows")]
    let binary_name = "fastpotify.exe";

    let new_binary = unpacked_dir.join(binary_name);
    log::debug!("new binary path: {new_binary:?}, is_file: {}", new_binary.is_file());
    if !new_binary.is_file() {
        // List the unpacked dir contents
        if let Ok(entries) = std::fs::read_dir(&unpacked_dir) {
            for entry in entries.flatten() {
                log::debug!("unpacked entry: {:?}", entry.file_name());
            }
        }
        anyhow::bail!("archive did not contain {binary_name} at the expected path");
    }

    // Replace the running binary. On Linux you cannot overwrite a running
    // binary's inode (ETXTBSY), so we copy to a temp path in the same
    // directory and `rename` it into place — the kernel keeps the old inode
    // alive for the running process but links the new file to the old path.
    let current_exe = std::env::current_exe()?;
    let exe_dir = current_exe.parent().unwrap_or(std::path::Path::new("."));
    let temp_exe = exe_dir.join(format!(".fastpotify-update-{}", std::process::id()));
    std::fs::copy(&new_binary, &temp_exe)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&temp_exe, std::fs::Permissions::from_mode(0o755))?;
    }
    std::fs::rename(&temp_exe, &current_exe)?;
    // Clean up the temp download.
    let _ = std::fs::remove_dir_all(&tmp);

    log::info!("updated Fastpotify binary to v{}", release.version);

    Ok(current_exe)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions_compare_numerically() {
        assert!(is_newer("0.1.4", "0.1.3"));
        assert!(is_newer("0.2.0", "0.1.9"));
        assert!(is_newer("1.0.0", "0.9.9"));
        assert!(is_newer("0.1.10", "0.1.9"));
        assert!(!is_newer("0.1.3", "0.1.3"));
        assert!(!is_newer("0.1.2", "0.1.3"));
        assert!(
            !is_newer("0.2.0-rc1", "0.1.3"),
            "pre-releases are not announced"
        );
        assert!(!is_newer("nightly", "0.1.3"));
        // A release candidate hears about its release, and nothing older.
        assert!(is_newer("0.4.0", "0.4.0-rc1"));
        assert!(is_newer("0.4.1", "0.4.0-rc1"));
        assert!(!is_newer("0.4.0-rc1", "0.4.0"));
        assert!(!is_newer("0.4.0-rc2", "0.4.0-rc1"));
        assert!(!is_newer("0.3.0", "0.4.0-rc1"));
    }
}
