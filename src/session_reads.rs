//! Catalogue reads over the streaming session, shaped as the Web API answers
//! them. The session speaks the protocol Spotify's own clients use, which has
//! no per-app quota, so a playlist someone else owns opens without waiting on
//! the shared app's rate limit.

use std::collections::{BTreeSet, HashMap};

use base64::Engine as _;
use librespot_core::{FileId, Session, error::ErrorKind, spotify_id::SpotifyId};
use librespot_metadata::artist::Artist as SessionArtist;
use librespot_metadata::image::Images;
use librespot_metadata::{Album as SessionAlbum, Episode as SessionEpisode, Track as SessionTrack};
use librespot_protocol::extended_metadata::{BatchedEntityRequest, EntityRequest, ExtensionQuery};
use librespot_protocol::extension_kind::ExtensionKind;
use librespot_protocol::playlist4_external::{ListAttributes, SelectedListContent};
use protobuf::{EnumOrUnknown, Message as _};
use reqwest::Method;

use crate::api::ApiError;
use crate::api::models::{
    Album, ArtistRef, Episode, Image, Owner, Page, PlayableItem, Playlist, PlaylistItem, Show,
    Track, TrackCount, UserRef,
};
use crate::util::uri_kind;

const IMAGE_HOST: &str = "https://i.scdn.co/image/";

/// Why the session could not answer a read.
#[derive(Debug)]
pub enum Failure {
    /// Spotify's answer is final and the Web API would give the same, so
    /// show it rather than spend the shared quota asking again.
    Definitive(ApiError),
    /// The session could not be reached or understood; the Web API may do
    /// better.
    Retry(anyhow::Error),
}

impl From<librespot_core::Error> for Failure {
    fn from(error: librespot_core::Error) -> Self {
        match error.kind {
            ErrorKind::NotFound => Self::Definitive(ApiError::Status {
                status: 404,
                message: "Spotify has no playlist by that id".into(),
            }),
            ErrorKind::PermissionDenied => Self::Definitive(ApiError::Status {
                status: 403,
                message: "this playlist is private".into(),
            }),
            _ => Self::Retry(error.into()),
        }
    }
}

impl From<protobuf::Error> for Failure {
    fn from(error: protobuf::Error) -> Self {
        Self::Retry(error.into())
    }
}

/// A playlist's header: name, owner, cover, and snapshot.
pub async fn playlist(session: &Session, id: &str) -> Result<Playlist, Failure> {
    let list = list(session, id, 0, 0).await?;
    let owner = list.owner_username();
    // `Playlist::owner_name` already reads a missing name as Spotify's.
    let owner_name = if owner == "spotify" {
        None
    } else {
        user_display_name(session, owner).await
    };
    Ok(header(id, &list, owner_name))
}

/// One page of a playlist's rows, as the Web API pages them.
pub async fn items(
    session: &Session,
    id: &str,
    offset: u32,
    limit: u32,
) -> Result<Page<PlaylistItem>, Failure> {
    let list = list(session, id, offset, limit).await?;
    let total = u32::try_from(list.length()).unwrap_or_default();
    let contents = list.contents.get_or_default();
    let rows = &contents.items[window(contents.items.len(), contents.pos(), offset, limit)];
    let uris: Vec<&str> = rows.iter().map(|row| row.uri()).collect();
    let mut playables = metadata(session, &uris).await?;
    let items = rows
        .iter()
        .map(|row| {
            let attributes = row.attributes.get_or_default();
            PlaylistItem {
                added_at: iso8601(attributes.timestamp()),
                added_by: non_empty(attributes.added_by()).map(|id| UserRef { id: Some(id) }),
                is_local: uri_kind(row.uri()) == Some("local"),
                item: playables.remove(row.uri()),
                ..Default::default()
            }
        })
        .collect();
    Ok(Page {
        items,
        total,
        limit,
        offset,
        // Only its presence is read.
        next: (offset.saturating_add(limit) < total).then(String::new),
    })
}

/// The display name behind a user id, from the profile view Spotify's
/// clients read; `None` when nothing answers.
pub async fn user_display_name(session: &Session, user_id: &str) -> Option<String> {
    let bytes = session
        .spclient()
        .get_user_profile(user_id, Some(0), Some(0))
        .await
        .ok()?;
    let json: serde_json::Value = serde_json::from_slice(&bytes).ok()?;
    json.get("name")
        .and_then(|value| value.as_str())
        .map(str::to_string)
}

/// The list's header and the `length` rows from `from`; none for the header
/// alone.
async fn list(
    session: &Session,
    id: &str,
    from: u32,
    length: u32,
) -> Result<SelectedListContent, Failure> {
    SpotifyId::from_base62(id)?;
    let endpoint = format!(
        "/playlist/v2/playlist/{id}?decorate=revision,length,attributes,owner&from={from}&length={length}"
    );
    let bytes = session
        .spclient()
        .request(&Method::GET, &endpoint, None, None)
        .await?;
    Ok(SelectedListContent::parse_from_bytes(&bytes)?)
}

fn header(id: &str, list: &SelectedListContent, owner_name: Option<String>) -> Playlist {
    let attributes = list.attributes.get_or_default();
    let owner = list.owner_username();
    Playlist {
        id: id.to_string(),
        name: attributes.name().to_string(),
        uri: format!("spotify:playlist:{id}"),
        description: non_empty(attributes.description()),
        images: playlist_images(attributes),
        owner: Owner {
            id: Some(owner.to_string()),
            display_name: owner_name,
            uri: Some(format!("spotify:user:{owner}")),
        },
        collaborative: attributes.collaborative(),
        snapshot_id: Some(snapshot(list.revision())),
        items_count: Some(TrackCount {
            total: u32::try_from(list.length()).unwrap_or_default(),
        }),
        ..Default::default()
    }
}

/// The Web API's snapshot id is the playlist revision in base64, so a
/// header read here matches one read there, and the disk cache with both.
fn snapshot(revision: &[u8]) -> String {
    base64::engine::general_purpose::STANDARD_NO_PAD.encode(revision)
}

/// The cover, from the sized pictures when the list carries them, else the
/// one picture every list has.
fn playlist_images(attributes: &ListAttributes) -> Vec<Image> {
    let sized: Vec<Image> = attributes
        .picture_size
        .iter()
        .filter_map(|picture| {
            Some(Image {
                url: image_url(picture.url())?,
                width: match picture.target_name() {
                    "small" => Some(60),
                    "default" => Some(300),
                    "large" => Some(640),
                    _ => None,
                },
                height: None,
            })
        })
        .collect();
    if !sized.is_empty() {
        return sized;
    }
    let picture = attributes.picture();
    if picture.is_empty() {
        return Vec::new();
    }
    vec![Image {
        url: file_url(&FileId::from_raw(picture)),
        width: None,
        height: None,
    }]
}

/// A picture reference as the list gives it: a web address, or an image
/// URI naming a file on Spotify's image host.
fn image_url(reference: &str) -> Option<String> {
    if reference.starts_with("https://") || reference.starts_with("http://") {
        return Some(reference.to_string());
    }
    reference
        .strip_prefix("spotify:image:")
        .map(|hex| format!("{IMAGE_HOST}{hex}"))
}

fn file_url(file: &FileId) -> String {
    format!("{IMAGE_HOST}{}", file.to_base16().unwrap_or_default())
}

/// The rows at `offset`, at most `limit` of them, within rows that start at
/// `pos`: the `from` asked for, or zero should Spotify send the whole list.
fn window(len: usize, pos: i32, offset: u32, limit: u32) -> std::ops::Range<usize> {
    let pos = u32::try_from(pos).unwrap_or_default();
    let start = (offset.saturating_sub(pos) as usize).min(len);
    let end = start.saturating_add(limit as usize).min(len);
    start..end
}

fn non_empty(text: &str) -> Option<String> {
    (!text.is_empty()).then(|| text.to_string())
}

/// Track and episode details for the given URIs, in one batched request.
/// A URI Spotify does not answer for is absent, as the Web API leaves a
/// row's item empty when it cannot be played.
async fn metadata(
    session: &Session,
    uris: &[&str],
) -> Result<HashMap<String, PlayableItem>, Failure> {
    let mut request = BatchedEntityRequest::new();
    // A playlist can hold the same song twice; ask for it once.
    for uri in uris.iter().copied().collect::<BTreeSet<_>>() {
        let kind = match uri_kind(uri) {
            Some("track") => ExtensionKind::TRACK_V4,
            Some("episode") => ExtensionKind::EPISODE_V4,
            _ => continue,
        };
        request.entity_request.push(EntityRequest {
            entity_uri: uri.to_string(),
            query: vec![ExtensionQuery {
                extension_kind: EnumOrUnknown::new(kind),
                ..Default::default()
            }],
            ..Default::default()
        });
    }
    let mut playables = HashMap::new();
    if request.entity_request.is_empty() {
        return Ok(playables);
    }
    let response = session.spclient().get_extended_metadata(request).await?;
    for array in response.extended_metadata {
        let Ok(kind) = array.extension_kind.enum_value() else {
            continue;
        };
        for data in array.extension_data {
            if let Some(item) = data
                .extension_data
                .as_ref()
                .and_then(|any| playable(kind, &any.value))
            {
                playables.insert(data.entity_uri, item);
            }
        }
    }
    Ok(playables)
}

fn playable(kind: ExtensionKind, bytes: &[u8]) -> Option<PlayableItem> {
    match kind {
        ExtensionKind::TRACK_V4 => {
            let message = librespot_protocol::metadata::Track::parse_from_bytes(bytes).ok()?;
            Some(PlayableItem::Track(track(
                SessionTrack::try_from(&message).ok()?,
            )))
        }
        ExtensionKind::EPISODE_V4 => {
            let message = librespot_protocol::metadata::Episode::parse_from_bytes(bytes).ok()?;
            Some(PlayableItem::Episode(episode(
                SessionEpisode::try_from(&message).ok()?,
            )))
        }
        _ => None,
    }
}

fn track(track: SessionTrack) -> Track {
    Track {
        id: track.id.to_id().ok(),
        uri: track.id.to_uri().unwrap_or_default(),
        name: track.name,
        duration_ms: u32::try_from(track.duration).unwrap_or_default(),
        explicit: track.is_explicit,
        artists: track.artists.iter().map(artist_ref).collect(),
        album: Some(album(track.album)),
        track_number: Some(u32::try_from(track.number).unwrap_or_default()),
        disc_number: Some(u32::try_from(track.disc_number).unwrap_or_default()),
        popularity: Some(track.popularity.clamp(0, 100) as u8),
        ..Default::default()
    }
}

fn album(album: SessionAlbum) -> Album {
    let covers = if album.covers.is_empty() {
        &album.cover_group
    } else {
        &album.covers
    };
    Album {
        id: album.id.to_id().unwrap_or_default(),
        uri: album.id.to_uri().unwrap_or_default(),
        images: images(covers),
        artists: album.artists.iter().map(artist_ref).collect(),
        label: non_empty(&album.label),
        name: album.name,
        ..Default::default()
    }
}

fn episode(episode: SessionEpisode) -> Episode {
    Episode {
        id: episode.id.to_id().unwrap_or_default(),
        uri: episode.id.to_uri().unwrap_or_default(),
        name: episode.name,
        duration_ms: u32::try_from(episode.duration).unwrap_or_default(),
        description: episode.description,
        images: images(&episode.covers),
        release_date: iso8601(episode.publish_time.as_timestamp_ms()),
        explicit: episode.is_explicit,
        show: Some(Show {
            name: episode.show_name,
            ..Default::default()
        }),
        ..Default::default()
    }
}

fn artist_ref(artist: &SessionArtist) -> ArtistRef {
    ArtistRef {
        id: artist.id.to_id().ok(),
        name: artist.name.clone(),
        uri: artist.id.to_uri().ok(),
    }
}

fn images(images: &Images) -> Vec<Image> {
    images
        .iter()
        .map(|image| Image {
            url: file_url(&image.id),
            width: u32::try_from(image.width).ok().filter(|width| *width > 0),
            height: u32::try_from(image.height)
                .ok()
                .filter(|height| *height > 0),
        })
        .collect()
}

/// Milliseconds since the epoch as the Web API writes a time; `None` for
/// the zero a list gives when it never recorded one.
fn iso8601(timestamp_ms: i64) -> Option<String> {
    (timestamp_ms > 0)
        .then(|| jiff::Timestamp::from_millisecond(timestamp_ms).ok())
        .flatten()
        .map(|time| time.strftime("%FT%TZ").to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn times_are_written_as_the_web_api_writes_them() {
        assert_eq!(
            iso8601(1_700_000_000_000).as_deref(),
            Some("2023-11-14T22:13:20Z")
        );
        assert_eq!(iso8601(0), None, "never recorded");
    }

    #[test]
    fn pictures_resolve_to_the_image_host() {
        assert_eq!(
            image_url("spotify:image:ab12").as_deref(),
            Some("https://i.scdn.co/image/ab12")
        );
        assert_eq!(
            image_url("https://mosaic.scdn.co/300/x").as_deref(),
            Some("https://mosaic.scdn.co/300/x")
        );
        assert_eq!(image_url("spotify:mosaic:x"), None);
    }

    #[test]
    fn rows_are_taken_relative_to_where_the_answer_starts() {
        // Spotify answered the `from` asked for.
        assert_eq!(window(50, 100, 100, 50), 0..50);
        // Spotify sent the whole list.
        assert_eq!(window(120, 0, 100, 50), 100..120);
        // Nothing past the end.
        assert_eq!(window(0, 500, 500, 50), 0..0);
    }

    #[test]
    fn the_header_reads_as_a_web_api_playlist() {
        let mut list = SelectedListContent::new();
        list.set_revision(vec![0, 0, 0, 7]);
        list.set_length(3);
        list.set_owner_username("someone".into());
        let attributes = list.attributes.mut_or_insert_default();
        attributes.set_name("Road trip".into());
        attributes.set_collaborative(true);
        attributes.set_picture(vec![0xab, 0xcd]);

        let playlist = header("pl1", &list, Some("Someone".into()));
        assert_eq!(playlist.name, "Road trip");
        assert_eq!(playlist.uri, "spotify:playlist:pl1");
        assert_eq!(playlist.description, None, "empty means none");
        assert!(playlist.collaborative);
        assert_eq!(playlist.owner.id.as_deref(), Some("someone"));
        assert_eq!(playlist.owner_name(), "Someone");
        assert_eq!(playlist.track_total(), 3);
        assert_eq!(playlist.snapshot_id.as_deref(), Some("AAAABw"));
        assert!(playlist.images[0].url.starts_with(IMAGE_HOST));
    }

    #[test]
    fn a_missing_playlist_is_final_and_a_dropped_line_is_not() {
        let missing = Failure::from(librespot_core::Error::not_found("gone"));
        assert!(matches!(
            missing,
            Failure::Definitive(ApiError::Status { status: 404, .. })
        ));
        let dropped = Failure::from(librespot_core::Error::unavailable("offline"));
        assert!(matches!(dropped, Failure::Retry(_)));
    }
}
