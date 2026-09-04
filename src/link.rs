//! Spotify links as they arrive from outside: the desktop's URL handler,
//! the command line, and a second launch handing one to the running
//! instance.
//!
//! Every shape Spotify hands out comes back as the one canonical URI,
//! `spotify:<kind>:<id>`, or nothing when it is not something the app can
//! open.

/// What a link may point at. Anything else, a user, a search, a station,
/// is not a page here.
const KINDS: [&str; 6] = ["track", "album", "artist", "playlist", "show", "episode"];

/// The canonical `spotify:<kind>:<id>` behind `text`, or `None` when it is
/// not a link to a track, album, artist, playlist, show, or episode.
///
/// Accepted: `spotify:track:ID`, the old `spotify:user:NAME:playlist:ID`,
/// `spotify://track/ID`, and `https://open.spotify.com/track/ID` with or
/// without a locale segment (`/intl-de/`), a query string, or the old
/// `/user/NAME/playlist/ID` shape.
pub fn parse(text: &str) -> Option<String> {
    let text = text.trim();
    let mut segments: Vec<&str> = if let Some(rest) = text.strip_prefix("spotify://") {
        // The URL shape of the URI: `spotify://track/ID`, or the web
        // address with its scheme swapped.
        let mut segments = path_segments(rest);
        if segments.first().is_some_and(|host| is_web_host(host)) {
            segments.remove(0);
        }
        segments
    } else if let Some(rest) = text.strip_prefix("spotify:") {
        rest.split(':').filter(|part| !part.is_empty()).collect()
    } else {
        let rest = text
            .strip_prefix("https://")
            .or_else(|| text.strip_prefix("http://"))?;
        let mut segments = path_segments(rest);
        if !segments.first().is_some_and(|host| is_web_host(host)) {
            return None;
        }
        segments.remove(0);
        segments
    };
    // Old playlist links carry the owner: spotify:user:NAME:playlist:ID.
    if segments.len() >= 4 && segments[0] == "user" && segments[2] == "playlist" {
        segments.drain(..2);
    }
    // The web address may start with a locale: open.spotify.com/intl-de/…
    if segments
        .first()
        .is_some_and(|first| first.starts_with("intl-"))
    {
        segments.remove(0);
    }
    let [kind, id, ..] = segments.as_slice() else {
        return None;
    };
    let kind = kind.to_ascii_lowercase();
    if !KINDS.contains(&kind.as_str()) || !is_id(id) {
        return None;
    }
    Some(format!("spotify:{kind}:{id}"))
}

/// The path of a web address split at slashes, its query and fragment
/// dropped, empty segments (a trailing slash) with them.
fn path_segments(rest: &str) -> Vec<&str> {
    let end = rest.find(['?', '#']).unwrap_or(rest.len());
    rest[..end]
        .split('/')
        .filter(|part| !part.is_empty())
        .collect()
}

fn is_web_host(host: &str) -> bool {
    matches!(
        host.to_ascii_lowercase().as_str(),
        "open.spotify.com" | "play.spotify.com"
    )
}

/// Spotify ids are base62; anything else on a link is not one, whatever
/// hands it over.
fn is_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= 64 && id.bytes().all(|byte| byte.is_ascii_alphanumeric())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every shape Spotify's own apps and site hand out lands on the one
    /// URI the app navigates by.
    #[test]
    fn every_link_shape_becomes_the_one_uri() {
        // #given / #when / #then
        for (link, uri) in [
            (
                "spotify:track:4uLU6hMCjMI75M1A2tKUQC",
                "spotify:track:4uLU6hMCjMI75M1A2tKUQC",
            ),
            (
                "spotify:playlist:37i9dQZF1DXcBWIGoYBM5M",
                "spotify:playlist:37i9dQZF1DXcBWIGoYBM5M",
            ),
            (
                "spotify:user:carmine:playlist:37i9dQZF1DXcBWIGoYBM5M",
                "spotify:playlist:37i9dQZF1DXcBWIGoYBM5M",
            ),
            (
                "spotify://album/1DFixLWuPkv3KT3TnV35m3",
                "spotify:album:1DFixLWuPkv3KT3TnV35m3",
            ),
            (
                "spotify://open.spotify.com/artist/4Z8W4fKeB5YxbusRsdQVPb",
                "spotify:artist:4Z8W4fKeB5YxbusRsdQVPb",
            ),
            (
                "https://open.spotify.com/track/4uLU6hMCjMI75M1A2tKUQC",
                "spotify:track:4uLU6hMCjMI75M1A2tKUQC",
            ),
            (
                "https://open.spotify.com/track/4uLU6hMCjMI75M1A2tKUQC?si=abc123&nd=1",
                "spotify:track:4uLU6hMCjMI75M1A2tKUQC",
            ),
            (
                "https://open.spotify.com/intl-it/album/1DFixLWuPkv3KT3TnV35m3/",
                "spotify:album:1DFixLWuPkv3KT3TnV35m3",
            ),
            (
                "https://open.spotify.com/user/carmine/playlist/37i9dQZF1DXcBWIGoYBM5M",
                "spotify:playlist:37i9dQZF1DXcBWIGoYBM5M",
            ),
            (
                "http://play.spotify.com/show/4rOoJ6Egrf8K2IrywzwOMk",
                "spotify:show:4rOoJ6Egrf8K2IrywzwOMk",
            ),
            (
                "https://OPEN.SPOTIFY.COM/episode/512ojhOuo1ktJprKbVcKyQ#top",
                "spotify:episode:512ojhOuo1ktJprKbVcKyQ",
            ),
            (
                "  spotify:Artist:4Z8W4fKeB5YxbusRsdQVPb\n",
                "spotify:artist:4Z8W4fKeB5YxbusRsdQVPb",
            ),
        ] {
            assert_eq!(parse(link).as_deref(), Some(uri), "{link}");
        }
    }

    /// What is not a page here, or not Spotify's at all, is refused rather
    /// than guessed at.
    #[test]
    fn what_is_not_a_page_is_refused() {
        // #given / #when / #then
        for link in [
            "",
            "spotify:",
            "spotify:track",
            "spotify:track:",
            "spotify:user:carmine",
            "spotify:search:rock",
            "spotify:station:track:4uLU6hMCjMI75M1A2tKUQC",
            "spotify:local:Artist:Album:Song:180",
            "spotify:track:4uLU6hMCjMI75M1A2tKUQC/../etc",
            "spotify:track:a b",
            "https://example.com/track/4uLU6hMCjMI75M1A2tKUQC",
            "https://open.spotify.com/",
            "https://open.spotify.com/user/carmine",
            "https://open.spotify.com/intl-it/",
            "file:///etc/passwd",
            "4uLU6hMCjMI75M1A2tKUQC",
        ] {
            assert_eq!(parse(link), None, "{link:?}");
        }
        let long = format!("spotify:track:{}", "x".repeat(65));
        assert_eq!(parse(&long), None);
    }
}
