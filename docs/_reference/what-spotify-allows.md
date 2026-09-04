---
title: What Spotify Lets a Client Do
description: What Fastpotify gets from Spotify's Web API and librespot, and what neither supports.
nav_order: 3
---

Fastpotify uses Spotify's
[Web API](https://developer.spotify.com/documentation/web-api) for account and
catalogue data. It uses [librespot](https://github.com/librespot-org/librespot)
for a few extra details and for audio playback.

Features missing from both cannot be added to Fastpotify. They may become
possible if Spotify adds an API or librespot adds lawful support.

## Web API

Fastpotify uses the Web API for:

- **Account:** profile details, followed artists, top artists and tracks, and
  the last fifty plays. Fastpotify keeps a longer local
  [history](/using-fastpotify/#recent).
- **Library:** playlists, saved tracks, albums, shows, and episodes. It can
  also save and remove items.
- **Playlists:** reading, creating, renaming, changing the description and
  visibility, adding and removing songs, reordering songs, and following and
  unfollowing. The API also supports cover uploads, but Fastpotify does not yet
  use them.
- **Catalogue:** albums, artists, tracks, shows, episodes, search, and
  recommendations. Artist pages include top tracks, releases, and related
  artists.
- **Playback control:** listing devices, transferring playback, play, pause,
  next, previous, seek, shuffle, repeat, volume, and reading or adding to the
  queue.

Users of the default app share Spotify's quota. Fastpotify limits heavy
requests and pauses a session when Spotify sends a `Retry-After` response.

Spotify also limits apps created since November 2024. These apps cannot access
Spotify-owned playlists, related artists, recommendations, or audio features.
This is why a personal app cannot handle every request and complete playlist
views still use the shared app. See [How It Connects](/how-it-connects/).

## librespot session

librespot signs in to Spotify and uses the same protocol as Spotify's own
clients. Fastpotify uses its session for:

- **Playlist folders and order.** Fastpotify can read them from Spotify's
  rootlist. librespot cannot create, rename, or move folders.
- **Playlist permissions.** The rootlist shows when a playlist shared by
  invitation can be edited. The Web API's `collaborative` flag does not cover
  these playlists. Fastpotify cannot manage collaborators.
- **Lyrics** when Spotify has them.
- **Display names** for the user IDs attached to songs in a playlist.
- **Song radio and autoplay** through Spotify's context resolver.

## librespot playback

librespot provides:

- Spotify catalogue playback at up to 320 kbps.
- Gapless playback, normalisation, and a local audio cache.
- Spotify Connect, so another Spotify client can transfer playback to this
  computer.
- Shuffle, repeat, seek, and volume.
- Songs and podcast episodes.

Spotify Premium is required. librespot cannot play audio with a free account.

Fastpotify uses a small librespot fork. Its patches add queue controls,
normalisation data for the visualisers, and an event for rejected audio keys.
They are listed in `Cargo.toml`. Larger changes go upstream first.

## Not available

The Web API and librespot do not provide these features:

- **Pins shared with the Spotify app.** Spotify stores pins in its private
  `your-library` service. There is no Web API for it, and librespot does not
  support its protocol. Fastpotify's pins are local and are stored in
  `settings.json`. See [issue #31](https://github.com/crmne/fastpotify/issues/31).
- **Editing playlist folders.** librespot can only read them.
- **Smart Shuffle, Jam, Blend, and similar Spotify features.** Spotify
  generates these for its own clients. Fastpotify only has plain shuffle.
- **Lossless audio.** librespot does not receive lossless streams. Fastpotify
  will reconsider this if librespot gains lawful support, but it will not
  bypass Spotify's DRM.
- **Local files.** librespot only streams Spotify's catalogue. It cannot fetch
  audio for a `spotify:local:` entry. Playing files from disk would require a
  separate player. See [issue #3](https://github.com/crmne/fastpotify/issues/3).
- **Audiobooks.** librespot does not play them.
- **Offline listening and downloads.** Spotify's DRM and the project's scope
  rule these out.
- **Playback speed and crossfade.** librespot supports neither. Fastpotify
  would have to add them to its own audio path.
- **Free-account playback.** Replacing Spotify audio with another source is
  also out of scope. See the
  [contribution guide](https://github.com/crmne/fastpotify/blob/main/CONTRIBUTING.md).
- **Friend activity, private-session status, and similar social features.**
  Spotify has no public API for them.
- **Canvas videos and video podcasts.** librespot does not provide them.
