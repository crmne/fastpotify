//! Piped-compatible HTTP search and stream lookup. No auth headers.

use anyhow::{Context, Result, anyhow};
use reqwest::Url;
use serde::Deserialize;
use std::time::Duration;

use super::matching::Candidate;
use super::streams::AudioStream;

const SEARCH_TIMEOUT: Duration = Duration::from_secs(12);

#[derive(Clone, Debug)]
pub struct PipedClient {
    http: reqwest::Client,
    base: Url,
}

impl PipedClient {
    pub fn new(base: &str) -> Result<Self> {
        let base = Url::parse(base).context("Piped API base URL is not valid")?;
        if base.scheme() != "http" && base.scheme() != "https" {
            anyhow::bail!("Piped API base URL must use http or https");
        }
        let http = reqwest::Client::builder()
            .user_agent(concat!("fastpotify/", env!("CARGO_PKG_VERSION")))
            .timeout(SEARCH_TIMEOUT)
            .connect_timeout(Duration::from_secs(8))
            .redirect(reqwest::redirect::Policy::limited(4))
            .build()
            .context("unable to build the Piped HTTP client")?;
        Ok(Self { http, base })
    }

    pub async fn search(&self, query: &str) -> Result<Vec<Candidate>> {
        let mut candidates = self.search_filter(query, "music_songs").await?;
        if candidates.is_empty() {
            candidates = self.search_filter(query, "videos").await?;
        }
        Ok(candidates)
    }

    async fn search_filter(&self, query: &str, filter: &str) -> Result<Vec<Candidate>> {
        let url = piped_url(&self.base, &["search"], &[("q", query), ("filter", filter)])?;
        let response = self
            .http
            .get(url)
            .send()
            .await
            .context("Piped search failed")?;
        let status = response.status();
        let bytes = response.bytes().await.context("Piped search body")?;
        if !status.is_success() {
            anyhow::bail!("Piped search returned HTTP {status}");
        }
        parse_search_body(&bytes)
    }

    pub async fn streams(&self, video_id: &str) -> Result<Vec<AudioStream>> {
        let id = sanitize_video_id(video_id).ok_or_else(|| anyhow!("invalid video id"))?;
        let url = piped_url(&self.base, &["streams", id], &[])?;
        let response = self
            .http
            .get(url)
            .send()
            .await
            .context("Piped streams failed")?;
        let status = response.status();
        let bytes = response.bytes().await.context("Piped streams body")?;
        if !status.is_success() {
            anyhow::bail!("Piped streams returned HTTP {status}");
        }
        parse_streams_body(&bytes)
    }
}

pub fn piped_url(base: &Url, segments: &[&str], query: &[(&str, &str)]) -> Result<Url> {
    let mut url = base.clone();
    {
        let mut path = url
            .path_segments_mut()
            .map_err(|_| anyhow!("Piped API base URL cannot be a base"))?;
        for segment in segments {
            if !segment.is_empty() {
                path.push(segment);
            }
        }
    }
    if !query.is_empty() {
        let mut pairs = url.query_pairs_mut();
        for (key, value) in query {
            pairs.append_pair(key, value);
        }
    }
    Ok(url)
}

pub fn sanitize_video_id(raw: &str) -> Option<&str> {
    let id = if let Some((_, rest)) = raw.split_once("v=") {
        rest.split('&').next().unwrap_or(rest)
    } else {
        let last = raw.rsplit(['/', '\\']).next().unwrap_or(raw);
        last.split('?').next().unwrap_or(last)
    };
    if id.len() == 11
        && id
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == '_' || ch == '-')
    {
        Some(id)
    } else {
        None
    }
}

#[derive(Debug, Deserialize)]
struct SearchResponse {
    #[serde(default)]
    items: Vec<SearchItem>,
}

#[derive(Debug, Deserialize)]
struct SearchItem {
    #[serde(default)]
    url: Option<String>,
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    title: Option<String>,
    #[serde(default, alias = "uploaderName", alias = "uploader")]
    uploader: Option<String>,
    #[serde(default)]
    duration: Option<serde_json::Value>,
    #[serde(default, rename = "type")]
    kind: Option<String>,
}

#[derive(Debug, Deserialize)]
struct StreamsResponse {
    #[serde(default, alias = "audioStreams")]
    audio_streams: Vec<PipedStream>,
}

#[derive(Debug, Deserialize)]
struct PipedStream {
    #[serde(default)]
    url: Option<String>,
    #[serde(default, alias = "mimeType")]
    mime_type: Option<String>,
    #[serde(default)]
    codec: Option<String>,
    #[serde(default)]
    format: Option<String>,
    #[serde(default)]
    bitrate: Option<u32>,
    #[serde(default, alias = "videoOnly")]
    video_only: Option<bool>,
    #[serde(default)]
    quality: Option<String>,
}

pub fn parse_search_body(bytes: &[u8]) -> Result<Vec<Candidate>> {
    if bytes.is_empty() {
        return Ok(Vec::new());
    }
    let items = if let Ok(response) = serde_json::from_slice::<SearchResponse>(bytes) {
        response.items
    } else {
        serde_json::from_slice::<Vec<SearchItem>>(bytes).context("Piped search JSON")?
    };
    let mut out = Vec::new();
    for item in items {
        if item
            .kind
            .as_deref()
            .is_some_and(|kind| kind != "stream" && kind != "video")
        {
            continue;
        }
        let raw_id = item
            .id
            .as_deref()
            .or(item.url.as_deref())
            .and_then(sanitize_video_id);
        let Some(id) = raw_id else {
            continue;
        };
        let title = item.title.unwrap_or_default();
        if title.is_empty() {
            continue;
        }
        out.push(Candidate {
            id: id.to_string(),
            title,
            uploader: item.uploader.unwrap_or_default(),
            duration_ms: duration_to_ms(item.duration),
        });
    }
    Ok(out)
}

pub fn parse_streams_body(bytes: &[u8]) -> Result<Vec<AudioStream>> {
    let response: StreamsResponse = serde_json::from_slice(bytes).context("Piped streams JSON")?;
    Ok(response
        .audio_streams
        .into_iter()
        .filter_map(|stream| {
            let url = stream.url.filter(|url| !url.is_empty())?;
            Some(AudioStream {
                url,
                mime: stream.mime_type,
                codec: stream.codec,
                format: stream.format,
                bitrate: stream.bitrate,
                video_only: stream.video_only.unwrap_or(false),
                quality: stream.quality,
                http_headers: Vec::new(),
            })
        })
        .collect())
}

fn duration_to_ms(value: Option<serde_json::Value>) -> Option<u32> {
    match value? {
        serde_json::Value::Number(number) => {
            let seconds = number.as_f64()?;
            if seconds <= 0.0 {
                None
            } else {
                Some((seconds * 1000.0) as u32)
            }
        }
        serde_json::Value::String(text) => parse_clock(&text),
        _ => None,
    }
}

fn parse_clock(text: &str) -> Option<u32> {
    let parts: Vec<u32> = text
        .split(':')
        .map(|part| part.parse().ok())
        .collect::<Option<Vec<_>>>()?;
    match parts.as_slice() {
        [seconds] => Some(*seconds * 1000),
        [minutes, seconds] => Some((minutes * 60 + seconds) * 1000),
        [hours, minutes, seconds] => Some((hours * 3600 + minutes * 60 + seconds) * 1000),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn search_json_object_and_array_parse() {
        let object = br#"{
            "items": [
                {
                    "url": "/watch?v=dQw4w9WgXcQ",
                    "title": "Song Title",
                    "uploaderName": "Artist - Topic",
                    "duration": 200,
                    "type": "stream"
                },
                {
                    "url": "/watch?v=nope",
                    "title": "A playlist",
                    "type": "playlist"
                }
            ]
        }"#;
        let parsed = parse_search_body(object).unwrap();
        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0].id, "dQw4w9WgXcQ");
        assert_eq!(parsed[0].duration_ms, Some(200_000));

        let array = br#"[{"id":"abcdefghijk","title":"Other","uploader":"X","duration":"3:05"}]"#;
        let parsed = parse_search_body(array).unwrap();
        assert_eq!(parsed[0].id, "abcdefghijk");
        assert_eq!(parsed[0].duration_ms, Some(185_000));
    }

    #[test]
    fn streams_json_maps_audio_only() {
        let json = br#"{
            "audioStreams": [
                {
                    "url": "https://cdn.example/a.m4a",
                    "mimeType": "audio/mp4",
                    "codec": "mp4a.40.2",
                    "format": "M4A",
                    "bitrate": 131072,
                    "videoOnly": false,
                    "quality": "128 kbps"
                },
                {
                    "url": "https://cdn.example/v.webm",
                    "mimeType": "audio/webm",
                    "codec": "opus",
                    "format": "WEBMA",
                    "bitrate": 160000
                }
            ]
        }"#;
        let streams = parse_streams_body(json).unwrap();
        assert_eq!(streams.len(), 2);
        assert_eq!(streams[0].format.as_deref(), Some("M4A"));
        assert!(!streams[0].video_only);
    }

    #[test]
    fn piped_url_is_path_safe() {
        let base = Url::parse("https://piped.example/api").unwrap();
        let url = piped_url(&base, &["streams", "dQw4w9WgXcQ"], &[]).unwrap();
        assert_eq!(
            url.as_str(),
            "https://piped.example/api/streams/dQw4w9WgXcQ"
        );
        let search = piped_url(&base, &["search"], &[("q", "a&b"), ("filter", "videos")]).unwrap();
        assert!(search.as_str().contains("q=a%26b"));
        assert!(sanitize_video_id("../etc/passwd").is_none());
        assert!(sanitize_video_id("dQw4w9WgXcQ").is_some());
    }
}
