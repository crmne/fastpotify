//! Daily update check against GitHub releases.

use std::time::Duration;

use anyhow::{Context, Result};
use serde::Deserialize;

const LATEST_RELEASE_URL: &str = "https://api.github.com/repos/crmne/fastpotify/releases/latest";

/// Update-check interval.
pub const CHECK_INTERVAL: Duration = Duration::from_secs(24 * 60 * 60);

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Release {
    /// The version number, without a leading `v`.
    pub version: String,
    /// The release page, with every download.
    pub url: String,
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
    Ok(
        is_newer(&version, env!("CARGO_PKG_VERSION")).then_some(Release {
            version,
            url: latest.html_url,
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
