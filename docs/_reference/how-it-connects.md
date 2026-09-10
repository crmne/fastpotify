---
title: How It Connects
description: Fastpotify's independent Spotify grants, what is stored, and how API traffic is routed.
nav_order: 1
---

## Independent grants, once each

Fastpotify uses separate credentials for Web API access, a personal app, and
local playback:

1. **The shared Web API app** keeps full catalogue and playlist coverage.
2. **Your optional personal Web API app** handles supported playback, library,
   catalog, playlist creation, and owned or collaborative playlist requests
   without using the shared app's quota. Complete playlist-library views and
   playlist-bearing search stay on the shared app so Spotify-owned results are
   not filtered out. Both Web API grants must verify as the same Spotify
   account.
3. **Local playback** uses
   [librespot](https://github.com/librespot-org/librespot). It needs one more
   browser approval and keeps an independent reusable credential. Spotify Premium
   is required.

Local playback authorization stays separate from both Web API grants. Its
browser approval requests only the streaming permission and always shows the
consent dialog. The playback session uses the account ID verified by either
Web API grant. A verified personal app can complete sign-in while the shared
app's verification is still waiting.

On `main`, for the release after 0.7.1, local playback retains the artist IDs
already supplied by librespot. Artist links in the player bar work before the
Web API's track metadata arrives, without an extra request.

On `main`, for the release after 0.7.1, requests that need a grant still being
verified wait for it instead of showing "not signed in". Sign-out cancels
pending requests, and their late results cannot undo a new sign-in. If Spotify
rejects a saved refresh grant, Fastpotify removes that grant and asks for a new
browser approval. Upgrading to protected storage does not itself require
signing in again.

By default, Fastpotify uses the public app shared with spotify-player, ncspot,
and Omarchy Spotify. Spotify divides its quota among all users. A personal app
adds a separate Development Mode quota. See
[Use a Personal Spotify App](/make-it-even-faster/).
On `main`, after 0.7.1, verified Premium accounts using shared access see a
one-time introduction to that option. Setup and dismissal are remembered in
settings. The prompt uses the existing account profile and adds no request.

On `main`, after 0.7.1, explicitly sorting a Library section loads its remaining
pages through the existing Web API grant, one at a time, while loaded entries
stay visible. A failed page stops automatic loading. Spotify custom playlist
order uses the existing account-scoped rootlist from local playback; sorting
and dragging never write that order back to Spotify.

## What the client stores

- On `main`, for the release after 0.7.1, shared and personal Web API grants
  and the reusable playback credential use the platform credential store:
  Secret Service on Linux, Keychain on macOS, and Credential Manager on
  Windows. Librespot retains its reusable credential in memory; Fastpotify
  owns persistence. Flatpak can talk to `org.freedesktop.secrets` for this.
  Version 0.7.1 still uses the older unencrypted files.
  See [migration, sign-out, and storage protection](/settings-and-files/).
- Downloaded audio and artwork, in the cache directory, within the budget
  you set.
- The first time MilkDrop opens with an empty preset folder, the two projectM
  preset packs are downloaded from GitHub (about 26 MB) and stored in the
  config directory.
- On Windows and macOS, desktop media controls load the cover themselves and
  are given a file, so the full-size artwork is downloaded into that cache
  when a song starts, even when no view on screen is showing it. Linux MPRIS
  carries the Spotify artwork URL for the desktop to resolve and asks for
  nothing extra.
- Lyrics, in the cache directory, for a month.
- Liked Songs metadata, scoped to the verified account, in the cache directory.
  This behavior is on `main`, for the release after 0.7.1.
  Cached pages less than 15 minutes old need no repeat request. Older cached
  prefixes refresh through the existing Web API grant, one page at a time,
  while the saved rows remain visible. Manual refresh starts immediately.
  Like and Unlike are kept over lagging reads until Spotify confirms them.
- Fastpotify has no telemetry, analytics, or hosted service. When the lyrics
  panel is open and Spotify has no lyrics, it sends the track's artist, title,
  album, and length to [lrclib.net](https://lrclib.net). It also checks
  api.github.com once a day for updates. You can turn off automatic checks in
  Settings, or request one there at any time. On macOS, **Check for Updates**
  is also in the application menu.

  On Windows and Linux, downloading an update fetches release metadata and
  `checksums.txt` from the project's GitHub release, then the matching binary
  archive, Windows installer, or universal macOS DMG. Fastpotify checks the published SHA-256 digest
  and the portable executable's reported version before offering a restart.
  Automatic downloads are optional; installation always waits for your click.
  Checks and downloads do not open the update popup. The green update pill opens
  it on request; closing the popup does not cancel a download.
  No Spotify credential is sent. These are GitHub-hosted checksums, not a
  separate publisher signature.

  Updates stage their files in a private `.fastpotify-update-*` directory beside
  the application so replacement stays on the same filesystem. The directory
  retains the previous executable or Mac app bundle and `result.txt` for recovery and diagnosis.
  Settings, caches and credential stores are not replaced. Package-manager
  installs keep their package-manager update path. Mac updates verify the bundle
  identifier, version and code signature before replacing the whole app bundle.
  A Developer ID installation also requires the same signing team and a passing
  macOS security assessment. Apps running from a disk image or an App Translocation
  directory must be moved to a writable installation directory first.

## When Spotify pushes back

Each Web API session has separate concurrency and rate limits. A `Retry-After`
response pauses only that session. Fastpotify routes each request once and
does not retry it through the other app.

Spotify can also explicitly refuse the key needed to decrypt a track. When
that happens, Fastpotify stops local playback and leaves the rest of the queue
alone instead of treating every following track as unavailable. This refusal
comes from Spotify; trying again later may work.

Before adding songs to an existing playlist, Fastpotify checks the rows it
already holds. A known duplicate produces an immediate confirmation naming the
song. Only a playlist that has not been fully loaded needs a background scan to
rule out duplicates. Once confirmed, the new rows appear locally at once.
On `main`, after 0.7.1,
a drop into an open editable playlist sends its chosen insertion position
through the same Web API grant. Duplicate checks and confirmation retain that
position; partial loaded pages keep the correct continuation offset. A
successful write advances the cached playlist to Spotify's returned snapshot
instead of downloading the playlist again. If Spotify cannot answer the scan,
Fastpotify preserves the requested edit and lets the write report its result.

## Receivers on the local network

Spotify's device list only shows signed-in receivers. A new librespot or
spotifyd receiver is therefore invisible to the Web API.

Receivers announce themselves over mDNS as `_spotify-connect._tcp` and answer
a small HTTP interface. Opening or refreshing the picker first reads
`getInfo` to find each receiver's name and device ID. These probes run off
the UI thread, four at a time, with a two-second limit per receiver and six
seconds overall after discovery. Only responding receivers with a name and
ID are offered. Matching IDs are combined; separate devices can have the
same name. These reads send no account credential.

When a receiver is selected, Fastpotify encrypts the stored librespot credential
with a receiver-specific key and a key from a Diffie-Hellman exchange. The
encrypted value only works for that receiver and exchange. Fastpotify does not
save another copy of the credential.

The receiver then signs in and appears in Spotify's device list. Fastpotify
uses the Web API for subsequent control requests.

## The engine

Playback runs on a separate runtime. Librespot maintains the Spotify Connect
session, exposes this computer as a device, receives transfers, and reports
playback state. If the session drops, it reconnects with the stored credential.

The engine discovers access points through `apresolve.spotify.com` and
connects over TCP in the resolver's preference order: port 4070 first,
falling back to 443 and 80. Only outbound connections are needed; no
inbound ports have to be open.

Each access-point attempt gives socket setup and the handshake a combined
five seconds. A stalled TCP connection or HTTP proxy tunnel therefore lets
librespot retry and move on to another endpoint instead of waiting for the
operating system's longer connection timeout.
