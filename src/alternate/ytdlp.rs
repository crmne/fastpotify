//! yt-dlp adapter. No cookies, accounts, bypass flags, or self-update.

use anyhow::{Context, Result, anyhow};
use serde::Deserialize;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;
use tokio::io::{AsyncReadExt, BufReader};
use tokio::process::Command;

use super::matching::Candidate;
use super::piped::sanitize_video_id;
use super::streams::AudioStream;

const MAX_STDOUT: usize = 8 * 1024 * 1024;
const SEARCH_TIMEOUT: Duration = Duration::from_secs(25);
const STREAM_TIMEOUT: Duration = Duration::from_secs(25);

#[derive(Clone, Debug)]
pub struct YtDlp {
    binary: PathBuf,
}

impl YtDlp {
    pub fn new(binary: PathBuf) -> Self {
        Self { binary }
    }

    pub async fn search(&self, query: &str) -> Result<Vec<Candidate>> {
        let query = sanitize_query(query);
        let stdout = run(
            &self.binary,
            &[
                "--ignore-config",
                "--no-update",
                "--no-warnings",
                "--no-cache-dir",
                "--no-playlist",
                "--flat-playlist",
                "--skip-download",
                "--dump-json",
                "--",
                &format!("ytsearch8:{query}"),
            ],
            SEARCH_TIMEOUT,
        )
        .await?;
        parse_search_json(&stdout)
    }

    pub async fn streams(&self, video_id: &str) -> Result<Vec<AudioStream>> {
        let id = sanitize_video_id(video_id).ok_or_else(|| anyhow!("invalid video id"))?;
        let watch = format!("https://www.youtube.com/watch?v={id}");
        let stdout = run(
            &self.binary,
            &[
                "--ignore-config",
                "--no-update",
                "--no-warnings",
                "--no-cache-dir",
                "--no-playlist",
                "--skip-download",
                "--dump-json",
                "--",
                &watch,
            ],
            STREAM_TIMEOUT,
        )
        .await?;
        parse_format_json(&stdout)
    }
}

fn sanitize_query(query: &str) -> String {
    query
        .chars()
        .filter(|ch| !ch.is_control())
        .take(180)
        .collect::<String>()
        .trim()
        .to_string()
}

async fn run(binary: &Path, args: &[&str], timeout: Duration) -> Result<String> {
    let mut command = Command::new(binary);
    command
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .stdin(Stdio::null());
    #[cfg(windows)]
    {
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        command.creation_flags(CREATE_NO_WINDOW);
    }
    let mut child = command.spawn().context("unable to start yt-dlp")?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| anyhow!("yt-dlp stdout"))?;
    let mut reader = BufReader::new(stdout);
    let mut buf = Vec::new();
    let read = async {
        let mut chunk = [0u8; 8192];
        loop {
            let n = reader.read(&mut chunk).await?;
            if n == 0 {
                break;
            }
            if buf.len() + n > MAX_STDOUT {
                anyhow::bail!("yt-dlp output was too large");
            }
            buf.extend_from_slice(&chunk[..n]);
        }
        Ok::<_, anyhow::Error>(())
    };
    tokio::select! {
        result = read => result?,
        _ = tokio::time::sleep(timeout) => {
            let _ = child.kill().await;
            anyhow::bail!("yt-dlp timed out");
        }
    }
    let status = child.wait().await.context("yt-dlp exit")?;
    if !status.success() {
        anyhow::bail!("yt-dlp exited with {status}");
    }
    String::from_utf8(buf).context("yt-dlp output was not UTF-8")
}

#[derive(Debug, Deserialize)]
struct FlatEntry {
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    uploader: Option<String>,
    #[serde(default)]
    channel: Option<String>,
    #[serde(default)]
    duration: Option<f64>,
}

#[derive(Debug, Deserialize)]
struct Dump {
    #[serde(default)]
    formats: Vec<Format>,
}

#[derive(Debug, Deserialize)]
struct Format {
    #[serde(default)]
    url: Option<String>,
    #[serde(default)]
    ext: Option<String>,
    #[serde(default)]
    acodec: Option<String>,
    #[serde(default)]
    vcodec: Option<String>,
    #[serde(default)]
    abr: Option<f64>,
    #[serde(default)]
    tbr: Option<f64>,
    #[serde(default)]
    audio_ext: Option<String>,
    #[serde(default)]
    http_headers: Option<HashMap<String, String>>,
}

pub fn parse_search_json(stdout: &str) -> Result<Vec<Candidate>> {
    let mut out = Vec::new();
    for line in stdout.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let entry: FlatEntry = serde_json::from_str(line).context("yt-dlp search JSON")?;
        let Some(id) = entry.id.as_deref().and_then(sanitize_video_id) else {
            continue;
        };
        let title = entry.title.unwrap_or_default();
        if title.is_empty() {
            continue;
        }
        out.push(Candidate {
            id: id.to_string(),
            title,
            uploader: entry.uploader.or(entry.channel).unwrap_or_default(),
            duration_ms: entry
                .duration
                .filter(|seconds| *seconds > 0.0)
                .map(|seconds| (seconds * 1000.0) as u32),
        });
    }
    Ok(out)
}

pub fn parse_format_json(stdout: &str) -> Result<Vec<AudioStream>> {
    let dump: Dump = serde_json::from_str(stdout.trim()).context("yt-dlp format JSON")?;
    Ok(dump
        .formats
        .into_iter()
        .filter_map(|format| {
            let url = format.url.filter(|url| !url.is_empty())?;
            let vcodec = format.vcodec.as_deref().unwrap_or("");
            let video_only = !vcodec.is_empty() && vcodec != "none";
            let bitrate = format.abr.or(format.tbr).map(|kbps| (kbps * 1000.0) as u32);
            Some(AudioStream {
                url,
                mime: None,
                codec: format.acodec,
                format: format.audio_ext.or(format.ext),
                bitrate,
                video_only,
                quality: None,
                http_headers: safe_http_headers(format.http_headers),
            })
        })
        .collect())
}

fn safe_http_headers(headers: Option<HashMap<String, String>>) -> Vec<(String, String)> {
    headers
        .unwrap_or_default()
        .into_iter()
        .filter(|(key, _)| {
            matches!(
                key.to_ascii_lowercase().as_str(),
                "user-agent" | "referer" | "origin"
            )
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn search_ndjson_parses_subset() {
        let stdout = r#"
{"id":"dQw4w9WgXcQ","title":"Never Gonna Give You Up","uploader":"RickAstleyVEVO","duration":213.0}
{"id":"bad","title":"nope"}
{"id":"abcdefghijk","title":"Other","channel":"Someone","duration":12}
"#;
        let parsed = parse_search_json(stdout).unwrap();
        assert_eq!(parsed.len(), 2);
        assert_eq!(parsed[0].id, "dQw4w9WgXcQ");
        assert_eq!(parsed[0].duration_ms, Some(213_000));
        assert_eq!(parsed[1].uploader, "Someone");
    }

    #[test]
    fn format_json_keeps_audio_and_flags_video() {
        let stdout = r#"{
            "id":"dQw4w9WgXcQ",
            "title":"x",
            "formats": [
                {"format_id":"140","url":"https://cdn.example/a.m4a","ext":"m4a","acodec":"mp4a.40.2","vcodec":"none","abr":128.0,"http_headers":{"User-Agent":"TestUA","Referer":"https://www.youtube.com/","Cookie":"secret","Authorization":"nope"}},
                {"format_id":"18","url":"https://cdn.example/v.mp4","ext":"mp4","acodec":"mp4a.40.2","vcodec":"avc1.42001E","tbr":500.0},
                {"format_id":"251","url":"https://cdn.example/a.webm","ext":"webm","acodec":"opus","vcodec":"none","abr":160.0}
            ]
        }"#;
        let streams = parse_format_json(stdout).unwrap();
        assert_eq!(streams.len(), 3);
        assert!(!streams[0].video_only);
        assert!(streams[1].video_only);
        assert_eq!(streams[0].format.as_deref(), Some("m4a"));
        assert_eq!(streams[0].bitrate, Some(128_000));
        assert!(
            streams[0]
                .http_headers
                .iter()
                .any(|(key, value)| key.eq_ignore_ascii_case("user-agent") && value == "TestUA")
        );
        assert!(
            streams[0]
                .http_headers
                .iter()
                .any(|(key, _)| key.eq_ignore_ascii_case("referer"))
        );
        assert!(
            streams[0]
                .http_headers
                .iter()
                .all(|(key, _)| !key.eq_ignore_ascii_case("cookie")
                    && !key.eq_ignore_ascii_case("authorization"))
        );
    }

    #[test]
    fn query_strips_controls() {
        assert_eq!(sanitize_query("hello\nworld\u{0}"), "helloworld");
    }
}
