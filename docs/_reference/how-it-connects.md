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
   browser approval and stores its own reusable credential. Spotify Premium
   is required.

Local playback authorization stays separate from both Web API grants.

By default, Fastpotify uses the public app shared with spotify-player, ncspot,
and Omarchy Spotify. Spotify divides its quota among all users. A personal app
adds a separate Development Mode quota. See
[Use a Personal Spotify App](/make-it-even-faster/).

## What the client stores

- Shared and personal Web API refresh tokens, plus librespot's credential, in
  the state directory with owner-only permissions
  ([file locations](/settings-and-files/)).
- Downloaded audio and artwork, in the cache directory, within the budget
  you set.
- The first time MilkDrop opens with an empty preset folder, the two projectM
  preset packs are downloaded from GitHub (about 26 MB) and stored in the
  config directory.
- On Windows and macOS, desktop media controls receive artwork from that
  cache instead of downloading the Spotify image a second time. Linux MPRIS
  carries the Spotify artwork URL for the desktop to resolve.
- Lyrics, in the cache directory, for a month.
- Fastpotify has no telemetry, analytics, or hosted service. When the lyrics
  panel is open and Spotify has no lyrics, it sends the track's artist, title,
  album, and length to [lrclib.net](https://lrclib.net). It also checks
  api.github.com once a day for updates. You can turn off update checks in
  Settings.

## When Spotify pushes back

Each Web API session has separate concurrency and rate limits. A `Retry-After`
response pauses only that session. Fastpotify routes each request once and
does not retry it through the other app.

Spotify can also explicitly refuse the key needed to decrypt a track. When
that happens, Fastpotify stops local playback and leaves the rest of the queue
alone instead of treating every following track as unavailable. This refusal
comes from Spotify; trying again later may work.

## Receivers on the local network

Spotify's device list only shows signed-in receivers. A new librespot or
spotifyd receiver is therefore invisible to the Web API.

Receivers announce themselves over mDNS as `_spotify-connect._tcp` and answer
a small HTTP interface. Fastpotify encrypts the stored librespot credential
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
