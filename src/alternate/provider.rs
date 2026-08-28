//! Piped-first resolver with yt-dlp fallback. Never sees Spotify tokens.

use anyhow::{Result, anyhow};
use std::future::Future;
use std::path::Path;
use std::pin::Pin;

use super::AlternateConfig;
use super::bundle;
use super::matching::Candidate;
use super::piped::PipedClient;
use super::streams::AudioStream;
use super::ytdlp::YtDlp;

pub type LookupFuture<T> = Pin<Box<dyn Future<Output = T> + Send>>;

pub trait MediaLookup: Send + Sync {
    fn search(&self, query: &str) -> LookupFuture<Result<(Vec<Candidate>, bool), String>>;
    fn streams(&self, id: &str) -> LookupFuture<Result<(Vec<AudioStream>, bool), String>>;
    fn canned_audio(&self) -> Option<Vec<u8>> {
        None
    }

    fn scripted_body(&self) -> Option<ScriptedBody> {
        None
    }
}

#[derive(Clone, Debug)]
pub struct ScriptedBody {
    pub chunks: Vec<Vec<u8>>,
    pub fail: Option<String>,
    pub content_length: Option<u64>,
    pub fail_after_ms: u64,
}

#[derive(Clone)]
pub struct Resolver {
    piped: Option<PipedClient>,
    ytdlp: Option<YtDlp>,
}

impl Resolver {
    pub fn from_config(config: &AlternateConfig, ytdlp_dir: &Path) -> Result<Self, String> {
        config.validate()?;
        let piped = config
            .piped_api_base
            .as_deref()
            .map(PipedClient::new)
            .transpose()
            .map_err(|error| error.to_string())?;
        let want_ytdlp =
            bundle::has_bundled_ytdlp() || bundle::user_ytdlp_present(config.ytdlp_path.as_deref());
        let ytdlp = if want_ytdlp {
            match bundle::resolve(config.ytdlp_path.as_deref(), ytdlp_dir) {
                Some(resolved) => {
                    bundle::log_choice(&resolved);
                    Some(YtDlp::new(resolved.path))
                }
                None => None,
            }
        } else {
            None
        };
        if piped.is_none() && ytdlp.is_none() {
            return Err(
                "Alternate playback needs a Piped-compatible API base URL, or a yt-dlp executable."
                    .into(),
            );
        }
        Ok(Self { piped, ytdlp })
    }

    pub async fn search(&self, query: &str) -> Result<(Vec<Candidate>, bool)> {
        if let Some(piped) = &self.piped {
            match piped.search(query).await {
                Ok(items) if !items.is_empty() => return Ok((items, false)),
                Ok(_) => log::info!("Piped search returned no candidates"),
                Err(error) => log::warn!("Piped search failed: {error}"),
            }
        }
        if let Some(ytdlp) = &self.ytdlp {
            return Ok((ytdlp.search(query).await?, true));
        }
        Err(anyhow!("no search provider answered"))
    }

    pub async fn streams(&self, video_id: &str) -> Result<(Vec<AudioStream>, bool)> {
        if let Some(piped) = &self.piped {
            match piped.streams(video_id).await {
                Ok(items) if !items.is_empty() => return Ok((items, false)),
                Ok(_) => log::info!("Piped streams returned none"),
                Err(error) => log::warn!("Piped streams failed: {error}"),
            }
        }
        if let Some(ytdlp) = &self.ytdlp {
            return Ok((ytdlp.streams(video_id).await?, true));
        }
        Err(anyhow!("no stream provider answered"))
    }
}

impl MediaLookup for Resolver {
    fn search(&self, query: &str) -> LookupFuture<Result<(Vec<Candidate>, bool), String>> {
        let this = self.clone();
        let query = query.to_string();
        Box::pin(async move {
            Resolver::search(&this, &query)
                .await
                .map_err(|error| error.to_string())
        })
    }

    fn streams(&self, id: &str) -> LookupFuture<Result<(Vec<AudioStream>, bool), String>> {
        let this = self.clone();
        let id = id.to_string();
        Box::pin(async move {
            Resolver::streams(&this, &id)
                .await
                .map_err(|error| error.to_string())
        })
    }
}
