//! Inspect a station without activating a Connect device or starting audio.
//! Run with `cargo run --locked --example radio_preview -- <track-id>`.

use librespot_core::{Session, SessionConfig, cache::Cache};

fn main() -> anyhow::Result<()> {
    let seed = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "3JA9Jsuxr4xgHXEawAdCp4".into());
    let dirs = fastpotify::paths::AppDirs::discover();
    let cache = Cache::new(Some(dirs.credentials_dir().as_path()), None, None, None)?;
    let credentials = cache
        .credentials()
        .ok_or_else(|| anyhow::anyhow!("No stored playback credential"))?;
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?
        .block_on(async move {
            let session = Session::new(SessionConfig::default(), Some(cache));
            session.connect(credentials, false).await?;
            let station = fastpotify::radio::resolve(&session, &seed).await?;
            println!(
                "{} Radio: {} tracks",
                station.seed.name,
                station.tracks.len()
            );
            for track in station.tracks.iter().take(5) {
                println!("{} | {} | {}", track.name, track.artist_names(), track.uri);
            }
            anyhow::Ok(())
        })
}
