//! Expand a LoadSpec into hydrated tracks via the Spotify Web API.

use std::collections::HashMap;

use anyhow::{Result, anyhow};

use crate::api::ApiClient;
use crate::api::models::{PlayableItem, Track};
use crate::player::{LoadSpec, LocalTrack};
use crate::util::{uri_id, uri_kind};

const MAX_TRACKS: usize = 200;

pub async fn expand_load(api: &ApiClient, spec: &LoadSpec) -> Result<Vec<LocalTrack>> {
    if spec.uris.iter().any(|uri| is_podcast(uri))
        || spec.context_uri.as_deref().is_some_and(is_podcast)
    {
        anyhow::bail!("Podcasts are not supported in alternate playback.");
    }
    let tracks = if !spec.uris.is_empty() {
        if let Some(known) = tracks_from_known(&spec.uris, &spec.known_tracks) {
            known
        } else {
            hydrate_uris(api, &spec.uris, &spec.known_tracks).await?
        }
    } else if let Some(context) = &spec.context_uri {
        expand_context(api, context).await?
    } else {
        anyhow::bail!("nothing to play");
    };
    let tracks: Vec<LocalTrack> = tracks
        .into_iter()
        .filter(|track| !track.is_episode)
        .collect();
    if tracks.is_empty() {
        anyhow::bail!("nothing to play");
    }
    Ok(tracks)
}

fn query_ready(track: &LocalTrack) -> bool {
    !track.uri.is_empty() && track.has_search_metadata()
}

/// The clicked/offset track, when the UI already has title and artist.
pub fn seed_queue(spec: &LoadSpec) -> Vec<LocalTrack> {
    let ready: HashMap<&str, &LocalTrack> = spec
        .known_tracks
        .iter()
        .filter(|track| query_ready(track))
        .map(|track| (track.uri.as_str(), track))
        .collect();
    let offset_uri = spec.offset_uri.as_deref().or_else(|| {
        let index = spec.offset_index.unwrap_or(0) as usize;
        spec.uris
            .get(index)
            .or(spec.uris.first())
            .map(String::as_str)
    });
    offset_uri
        .and_then(|uri| ready.get(uri).copied())
        .cloned()
        .into_iter()
        .collect()
}

fn tracks_from_known(uris: &[String], known: &[LocalTrack]) -> Option<Vec<LocalTrack>> {
    if uris.is_empty() {
        return None;
    }
    let ready: HashMap<&str, &LocalTrack> = known
        .iter()
        .filter(|track| query_ready(track))
        .map(|track| (track.uri.as_str(), track))
        .collect();
    let mut out = Vec::new();
    for uri in uris.iter().take(MAX_TRACKS) {
        if is_podcast(uri) || uri_kind(uri) != Some("track") {
            continue;
        }
        out.push((*ready.get(uri.as_str())?).clone());
    }
    (!out.is_empty()).then_some(out)
}

pub fn offset_index(spec: &LoadSpec, tracks: &[LocalTrack]) -> usize {
    if let Some(uri) = &spec.offset_uri
        && let Some(index) = tracks.iter().position(|track| &track.uri == uri)
    {
        return index;
    }
    spec.offset_index
        .map(|index| index as usize)
        .unwrap_or(0)
        .min(tracks.len().saturating_sub(1))
}

async fn hydrate_uris(
    api: &ApiClient,
    uris: &[String],
    known: &[LocalTrack],
) -> Result<Vec<LocalTrack>> {
    let mut known_by_uri: HashMap<String, LocalTrack> = known
        .iter()
        .filter(|track| query_ready(track))
        .cloned()
        .map(|track| (track.uri.clone(), track))
        .collect();
    let mut ids = Vec::new();
    let mut keep = Vec::new();
    for uri in uris.iter().take(MAX_TRACKS) {
        if is_podcast(uri) {
            continue;
        }
        if uri_kind(uri) != Some("track") {
            continue;
        }
        let Some(id) = uri_id(uri) else {
            continue;
        };
        keep.push(uri.clone());
        if !known_by_uri.contains_key(uri) {
            ids.push(id.to_string());
        }
    }
    if keep.is_empty() {
        anyhow::bail!("nothing to play");
    }
    if !ids.is_empty() {
        let fetched = api.tracks(&ids).await.map_err(|error| anyhow!("{error}"))?;
        for track in fetched {
            let local = local_from_track(&track);
            if !local.uri.is_empty() {
                known_by_uri.entry(local.uri.clone()).or_insert(local);
            }
        }
    }
    Ok(keep
        .into_iter()
        .map(|uri| {
            known_by_uri.remove(&uri).unwrap_or(LocalTrack {
                uri,
                ..LocalTrack::default()
            })
        })
        .filter(|track| !track.title.is_empty() || !track.uri.is_empty())
        .collect())
}

async fn expand_context(api: &ApiClient, context: &str) -> Result<Vec<LocalTrack>> {
    if is_liked_collection(context) {
        return liked_songs(api).await;
    }
    let kind = uri_kind(context).ok_or_else(|| anyhow!("unsupported context: {context}"))?;
    let id = uri_id(context).ok_or_else(|| anyhow!("unsupported context: {context}"))?;
    match kind {
        "playlist" => playlist_tracks(api, id).await,
        "album" => album_tracks(api, id).await,
        "artist" => {
            let name = api
                .artist(id)
                .await
                .map(|artist| artist.name)
                .unwrap_or_default();
            let tracks = api
                .artist_top_tracks(id, &name)
                .await
                .map_err(|error| anyhow!("{error}"))?;
            Ok(tracks.iter().map(local_from_track).collect())
        }
        "show" | "episode" => {
            anyhow::bail!("Podcasts are not supported in alternate playback.")
        }
        _ => anyhow::bail!(
            "Alternate playback cannot expand this context ({kind}). Play a track list, album, playlist, or artist instead."
        ),
    }
}

fn is_liked_collection(uri: &str) -> bool {
    uri.starts_with("spotify:user:") && uri.ends_with(":collection")
        || uri == "spotify:collection:tracks"
}

fn is_podcast(uri: &str) -> bool {
    matches!(uri_kind(uri), Some("show") | Some("episode"))
}

async fn playlist_tracks(api: &ApiClient, id: &str) -> Result<Vec<LocalTrack>> {
    let mut offset = 0u32;
    let mut out = Vec::new();
    while out.len() < MAX_TRACKS {
        let page = api
            .playlist_items(id, offset, 100)
            .await
            .map_err(|error| anyhow!("{error}"))?;
        let received = page.items.len() as u32;
        for item in page.items {
            if let Some(PlayableItem::Track(track)) = item.playable().cloned() {
                out.push(local_from_track(&track));
                if out.len() >= MAX_TRACKS {
                    break;
                }
            }
        }
        if received == 0 || page.next.is_none() {
            break;
        }
        offset += received;
    }
    Ok(out)
}

async fn album_tracks(api: &ApiClient, id: &str) -> Result<Vec<LocalTrack>> {
    let album = api.album(id).await.ok();
    let art = album
        .as_ref()
        .and_then(|album| crate::api::models::pick_image(&album.images, 640).map(str::to_string));
    let art_small = album
        .as_ref()
        .and_then(|album| crate::api::models::pick_image(&album.images, 64).map(str::to_string));
    let album_name = album
        .as_ref()
        .map(|album| album.name.clone())
        .unwrap_or_default();
    let mut offset = 0u32;
    let mut out = Vec::new();
    while out.len() < MAX_TRACKS {
        let page = api
            .album_tracks(id, offset, 50)
            .await
            .map_err(|error| anyhow!("{error}"))?;
        let received = page.items.len() as u32;
        for mut track in page.items {
            if track.album.is_none()
                && let Some(album) = &album
            {
                track.album = Some(album.clone());
            }
            let mut local = local_from_track(&track);
            if local.album.is_empty() {
                local.album = album_name.clone();
            }
            if local.art_url.is_none() {
                local.art_url = art.clone();
            }
            if local.art_small_url.is_none() {
                local.art_small_url = art_small.clone();
            }
            out.push(local);
            if out.len() >= MAX_TRACKS {
                break;
            }
        }
        if received == 0 || page.next.is_none() {
            break;
        }
        offset += received;
    }
    Ok(out)
}

async fn liked_songs(api: &ApiClient) -> Result<Vec<LocalTrack>> {
    let mut offset = 0u32;
    let mut out = Vec::new();
    while out.len() < MAX_TRACKS {
        let page = api
            .saved_tracks(offset, 50)
            .await
            .map_err(|error| anyhow!("{error}"))?;
        let received = page.items.len() as u32;
        for item in page.items {
            out.push(local_from_track(&item.track));
            if out.len() >= MAX_TRACKS {
                break;
            }
        }
        if received == 0 || page.next.is_none() {
            break;
        }
        offset += received;
    }
    Ok(out)
}

pub fn local_from_track(track: &Track) -> LocalTrack {
    LocalTrack {
        uri: if track.uri.is_empty() {
            track
                .id
                .as_ref()
                .map(|id| format!("spotify:track:{id}"))
                .unwrap_or_default()
        } else {
            track.uri.clone()
        },
        title: track.name.clone(),
        artists: track
            .artists
            .iter()
            .map(|artist| artist.name.clone())
            .collect(),
        album: track
            .album
            .as_ref()
            .map(|album| album.name.clone())
            .unwrap_or_default(),
        art_url: track.image(640).map(str::to_string),
        art_small_url: track.image(64).map(str::to_string),
        duration_ms: track.duration_ms,
        is_episode: false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_uris_skip_when_metadata_is_ready() {
        let known = vec![LocalTrack {
            uri: "spotify:track:a".into(),
            title: "Song".into(),
            artists: vec!["Artist".into()],
            duration_ms: 1_000,
            ..LocalTrack::default()
        }];
        let uris = vec!["spotify:track:a".into()];
        let got = tracks_from_known(&uris, &known).unwrap();
        assert_eq!(got[0].title, "Song");
        assert!(tracks_from_known(&uris, &[]).is_none());
        let incomplete = vec![LocalTrack {
            uri: "spotify:track:a".into(),
            title: String::new(),
            artists: vec!["Artist".into()],
            ..LocalTrack::default()
        }];
        assert!(tracks_from_known(&uris, &incomplete).is_none());
    }

    #[test]
    fn seed_queue_uses_offset_track() {
        let spec = LoadSpec {
            context_uri: Some("spotify:playlist:p".into()),
            offset_uri: Some("spotify:track:b".into()),
            known_tracks: vec![LocalTrack {
                uri: "spotify:track:b".into(),
                title: "B".into(),
                artists: vec!["Artist".into()],
                duration_ms: 2_000,
                ..LocalTrack::default()
            }],
            ..LoadSpec::default()
        };
        let seed = seed_queue(&spec);
        assert_eq!(seed.len(), 1);
        assert_eq!(seed[0].title, "B");
        let spec = LoadSpec {
            uris: vec!["spotify:track:a".into(), "spotify:track:b".into()],
            offset_index: Some(1),
            known_tracks: spec.known_tracks.clone(),
            ..LoadSpec::default()
        };
        assert_eq!(seed_queue(&spec)[0].uri, "spotify:track:b");
        let spec = LoadSpec {
            uris: vec!["spotify:track:a".into()],
            ..LoadSpec::default()
        };
        assert!(seed_queue(&spec).is_empty());
    }

    #[test]
    fn offset_prefers_uri_then_index() {
        let tracks = vec![
            track("spotify:track:a"),
            track("spotify:track:b"),
            track("spotify:track:c"),
        ];
        let spec = LoadSpec {
            offset_uri: Some("spotify:track:c".into()),
            offset_index: Some(0),
            ..LoadSpec::default()
        };
        assert_eq!(offset_index(&spec, &tracks), 2);
        let spec = LoadSpec {
            offset_index: Some(1),
            ..LoadSpec::default()
        };
        assert_eq!(offset_index(&spec, &tracks), 1);
    }

    #[test]
    fn podcast_uris_are_detected() {
        assert!(is_podcast("spotify:episode:x"));
        assert!(is_podcast("spotify:show:x"));
        assert!(!is_podcast("spotify:track:x"));
        assert!(is_liked_collection("spotify:user:me:collection"));
    }

    fn track(uri: &str) -> LocalTrack {
        LocalTrack {
            uri: uri.into(),
            title: uri.into(),
            ..LocalTrack::default()
        }
    }
}
