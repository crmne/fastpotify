//! Album art: fetched once, kept on disk, decoded by egui on demand.
//!
//! [`ArtLoader`] plugs into egui's image pipeline as a bytes loader for
//! `http(s)` URIs, so every view simply asks for `ui.image(url)`. The first
//! request for a URL starts a background download (or a disk-cache read);
//! until it lands egui shows a placeholder. Entries that no view has drawn
//! for a while are evicted so a long browsing session does not accumulate
//! textures without bound.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use egui::load::{Bytes, BytesLoadResult, BytesLoader, BytesPoll, LoadError};
use sha1::{Digest, Sha1};

const STALE_AFTER: Duration = Duration::from_secs(150);
const MAX_ART_BYTES: usize = 8 * 1024 * 1024;

enum Entry {
    Pending,
    Ready {
        bytes: Arc<[u8]>,
        last_used: Instant,
    },
    Failed(String),
}

struct Inner {
    entries: Mutex<HashMap<String, Entry>>,
    http: reqwest::Client,
    runtime: tokio::runtime::Handle,
    cache_dir: PathBuf,
}

#[derive(Clone)]
pub struct ArtLoader {
    inner: Arc<Inner>,
}

impl ArtLoader {
    pub fn new(http: reqwest::Client, runtime: tokio::runtime::Handle, cache_dir: PathBuf) -> Self {
        let _ = std::fs::create_dir_all(&cache_dir);
        Self {
            inner: Arc::new(Inner {
                entries: Mutex::new(HashMap::new()),
                http,
                runtime,
                cache_dir,
            }),
        }
    }

    /// Bytes for `url`, from memory, disk, or the network.
    pub async fn fetch(&self, url: &str) -> Result<Arc<[u8]>, String> {
        self.inner.fetch(url).await
    }

    /// Drops artwork no view has drawn recently, freeing bytes and textures.
    pub fn evict_stale(&self, ctx: &egui::Context) {
        let stale: Vec<String> = {
            let entries = self.inner.entries.lock().unwrap_or_else(|p| p.into_inner());
            entries
                .iter()
                .filter_map(|(url, entry)| match entry {
                    Entry::Ready { last_used, .. } if last_used.elapsed() > STALE_AFTER => {
                        Some(url.clone())
                    }
                    Entry::Failed(_) => Some(url.clone()),
                    _ => None,
                })
                .collect()
        };
        for url in stale {
            ctx.forget_image(&url);
        }
    }

    /// The disk-cache file holding `url`'s artwork, once it has been fetched.
    ///
    /// The cache is written atomically (a `.part` file, then a rename), so a
    /// file that is here at all holds a complete, successful response. The
    /// desktop media controls hand this path to the platform instead of the
    /// remote URL: macOS loads cover art itself, synchronously, inside a
    /// callback that cannot report a failure.
    pub fn cached_file(&self, url: &str) -> Option<PathBuf> {
        let path = self.inner.cache_path(url);
        std::fs::metadata(&path)
            .is_ok_and(|meta| meta.is_file() && meta.len() > 0)
            .then_some(path)
    }

    pub fn clear_disk_cache(&self) -> std::io::Result<u64> {
        let mut removed = 0;
        for entry in std::fs::read_dir(&self.inner.cache_dir)? {
            let entry = entry?;
            if entry.file_type()?.is_file() {
                removed += entry.metadata().map(|m| m.len()).unwrap_or(0);
                let _ = std::fs::remove_file(entry.path());
            }
        }
        Ok(removed)
    }
}

impl Inner {
    fn cache_path(&self, url: &str) -> PathBuf {
        let digest = Sha1::digest(url.as_bytes());
        let mut name = String::with_capacity(40);
        for byte in digest {
            use std::fmt::Write;
            let _ = write!(name, "{byte:02x}");
        }
        self.cache_dir.join(name)
    }

    async fn fetch(self: &Arc<Self>, url: &str) -> Result<Arc<[u8]>, String> {
        if let Some(Entry::Ready { bytes, .. }) = self
            .entries
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .get(url)
        {
            return Ok(Arc::clone(bytes));
        }
        let path = self.cache_path(url);
        let cached = tokio::task::spawn_blocking({
            let path = path.clone();
            move || std::fs::read(path).ok()
        })
        .await
        .ok()
        .flatten();
        let bytes: Vec<u8> = match cached {
            Some(bytes) if !bytes.is_empty() => bytes,
            _ => {
                let response = self
                    .http
                    .get(url)
                    .send()
                    .await
                    .map_err(|error| error.to_string())?;
                if !response.status().is_success() {
                    return Err(format!("artwork request failed: {}", response.status()));
                }
                let bytes = response.bytes().await.map_err(|error| error.to_string())?;
                if bytes.len() > MAX_ART_BYTES {
                    return Err("artwork is too large".to_string());
                }
                let bytes = bytes.to_vec();
                let write_path = path.clone();
                let payload = bytes.clone();
                self.runtime.spawn_blocking(move || {
                    let temporary = write_path.with_extension("part");
                    if std::fs::write(&temporary, &payload).is_ok() {
                        let _ = std::fs::rename(&temporary, &write_path);
                    }
                });
                bytes
            }
        };
        Ok(Arc::from(bytes))
    }

    fn start(self: &Arc<Self>, ctx: &egui::Context, url: String) {
        let loader = Arc::clone(self);
        let ctx = ctx.clone();
        self.runtime.spawn(async move {
            let result = loader.fetch(&url).await;
            let entry = match result {
                Ok(bytes) => Entry::Ready {
                    bytes,
                    last_used: Instant::now(),
                },
                Err(error) => Entry::Failed(error),
            };
            loader
                .entries
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .insert(url, entry);
            ctx.request_repaint();
        });
    }
}

impl BytesLoader for ArtLoader {
    fn id(&self) -> &'static str {
        "fastpotify::ArtLoader"
    }

    fn load(&self, ctx: &egui::Context, uri: &str) -> BytesLoadResult {
        if !(uri.starts_with("https://") || uri.starts_with("http://")) {
            return Err(LoadError::NotSupported);
        }
        let mut entries = self.inner.entries.lock().unwrap_or_else(|p| p.into_inner());
        match entries.get_mut(uri) {
            Some(Entry::Ready { bytes, last_used }) => {
                *last_used = Instant::now();
                Ok(BytesPoll::Ready {
                    size: None,
                    bytes: Bytes::Shared(Arc::clone(bytes)),
                    mime: None,
                })
            }
            Some(Entry::Pending) => Ok(BytesPoll::Pending { size: None }),
            Some(Entry::Failed(error)) => Err(LoadError::Loading(error.clone())),
            None => {
                entries.insert(uri.to_string(), Entry::Pending);
                drop(entries);
                self.inner.start(ctx, uri.to_string());
                Ok(BytesPoll::Pending { size: None })
            }
        }
    }

    fn forget(&self, uri: &str) {
        self.inner
            .entries
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .remove(uri);
    }

    fn forget_all(&self) {
        self.inner
            .entries
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .clear();
    }

    fn byte_size(&self) -> usize {
        self.inner
            .entries
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .values()
            .map(|entry| match entry {
                Entry::Ready { bytes, .. } => bytes.len(),
                _ => 0,
            })
            .sum()
    }
}

/// A colour that represents an album cover, suitable for tinting a dark or
/// light surface: the most common saturated hue, with its lightness pulled
/// into a range that still reads as a background.
pub fn accent_color(bytes: &[u8]) -> Option<[u8; 3]> {
    let decoded = image::load_from_memory(bytes).ok()?;
    let small = decoded.thumbnail(48, 48).to_rgb8();
    let mut buckets: HashMap<(u8, u8, u8), (u64, [u64; 3])> = HashMap::new();
    for pixel in small.pixels() {
        let [r, g, b] = pixel.0;
        let (max, min) = (r.max(g).max(b) as f32, r.min(g).min(b) as f32);
        let saturation = if max == 0.0 { 0.0 } else { (max - min) / max };
        let lightness = (max + min) / 510.0;
        // Weight toward vivid mid-tones so black borders and white text lose.
        let weight = (1.0 + saturation * 6.0) * (1.0 - (lightness - 0.5).abs() * 1.4).max(0.05);
        let weight = (weight * 100.0) as u64;
        let key = (r >> 4, g >> 4, b >> 4);
        let bucket = buckets.entry(key).or_insert((0, [0, 0, 0]));
        bucket.0 += weight;
        bucket.1[0] += r as u64 * weight;
        bucket.1[1] += g as u64 * weight;
        bucket.1[2] += b as u64 * weight;
    }
    let (_, (weight, sum)) = buckets.into_iter().max_by_key(|(_, (weight, _))| *weight)?;
    if weight == 0 {
        return None;
    }
    Some([
        (sum[0] / weight) as u8,
        (sum[1] / weight) as u8,
        (sum[2] / weight) as u8,
    ])
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The media controls ask for a file rather than a URL, and have to be
    /// told "not yet" rather than handed a path to nothing: macOS loads cover
    /// art itself and dereferences a failed load without checking it, which
    /// takes the whole process with it.
    #[test]
    fn a_cached_file_is_named_only_once_it_is_really_there() {
        let dir = std::env::temp_dir().join(format!("fastpotify-art-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let runtime = tokio::runtime::Builder::new_current_thread()
            .build()
            .expect("a runtime to hand the loader");
        let loader = ArtLoader::new(
            reqwest::Client::new(),
            runtime.handle().clone(),
            dir.clone(),
        );
        let url = "https://i.scdn.co/image/abc";

        assert_eq!(loader.cached_file(url), None, "nothing downloaded yet");

        // A half-written download never appears under its real name -- the
        // cache renames one into place -- but an empty file is not artwork.
        let path = loader.inner.cache_path(url);
        std::fs::write(&path, b"").expect("an empty file");
        assert_eq!(loader.cached_file(url), None, "empty is not artwork");

        std::fs::write(&path, b"\xff\xd8\xff jpeg-ish").expect("a file with bytes");
        assert_eq!(loader.cached_file(url), Some(path));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn accent_color_finds_dominant_hue() {
        let mut image = image::RgbImage::new(16, 16);
        for (x, _, pixel) in image.enumerate_pixels_mut() {
            *pixel = if x < 12 {
                image::Rgb([20, 120, 200])
            } else {
                image::Rgb([255, 255, 255])
            };
        }
        let mut bytes = Vec::new();
        image
            .write_to(
                &mut std::io::Cursor::new(&mut bytes),
                image::ImageFormat::Png,
            )
            .unwrap();
        let color = accent_color(&bytes).unwrap();
        assert!(
            color[2] > color[0],
            "expected the blue field, got {color:?}"
        );
    }
}
