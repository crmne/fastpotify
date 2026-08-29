---
title: How It Connects
description: Fastpotify's independent Spotify grants, what is stored, and how API traffic is routed.
nav_order: 1
---

## Independent grants, once each

Fastpotify uses independent credentials for Web API coverage, optional
personal acceleration, and local playback:

1. **The shared Web API app** provides broad catalog and playlist coverage.
2. **Your optional personal Web API app** handles supported playback, library,
   catalog, playlist creation, and owned or collaborative playlist requests
   without using the shared app's quota. Complete playlist-library views and
   playlist-bearing search stay on the shared app so Spotify-owned results are
   not filtered out. Both Web API grants must verify as the same Spotify
   account.
3. **Streaming** is actually playing audio on this computer, through
   [librespot](https://github.com/librespot-org/librespot). This runs the
   same browser flow once against Spotify's streaming client identity, after
   which librespot stores its own reusable credential. Premium is required,
   because that is what Spotify's streaming protocol requires.

Local playback authorization stays separate from both Web API grants.

By default the Web API uses the shared public application also used by
spotify-player, ncspot, and Omarchy Spotify, whose allowance Spotify
divides among everyone running any of them. An application of your own adds a
separate Development Mode quota; [Make It Even Faster](/make-it-even-faster/)
shows how to add one.

## What the client stores

- The shared and personal Web API refresh tokens and librespot's reusable credential, owner-only,
  in the state directory ([where](/settings-and-files/)).
- Downloaded audio and artwork, in the cache directory, within the budget
  you set.
- Lyrics, in the cache directory, for a month.
- Fastpotify has no telemetry, analytics, or hosted service. Besides Spotify
  and its album art CDN, the app contacts
  [lrclib.net](https://lrclib.net) while the lyrics panel is open and
  Spotify itself has no words for the track, sending the track's artist,
  title, album, and length, and to
  api.github.com once a day to learn whether a newer release exists, which
  Settings can turn off.

## When Spotify pushes back

Each Web API session has its own concurrency and cooldown. Fastpotify honours
`Retry-After` without pausing the other session and treats Development Mode
quota exhaustion separately from an ordinary burst limit. A logical request
is routed once before dispatch and is never retried through the other app.

## Receivers on the local network

Spotify's device list contains only receivers already logged in to the
account. Anything waiting to be given one, which is the normal state for a
self-hosted librespot or spotifyd, is invisible to the Web API.

Those receivers announce themselves over mDNS as `_spotify-connect._tcp` and
answer a small HTTP interface. Fastpotify asks a receiver to describe itself,
then hands over the reusable credential librespot already stores, wrapped
twice: once in a key derived from the receiver's own device id, and again in
a key both sides derive from a Diffie-Hellman exchange with the public key
that receiver just published. The encrypted value is specific to that
receiver and exchange. Fastpotify does not write another copy of the
credential.

The receiver then signs in and appears in Spotify's device list. Fastpotify
uses the Web API for subsequent control requests.

## The engine

Playback runs on a dedicated runtime: librespot maintains the Spotify
Connect session, exposes this computer as a device, receives transfers, and
reports its playback position. If the session drops, the engine reconnects
with the stored credential. This work does not block the interface.

The engine discovers access points through `apresolve.spotify.com` and
connects over TCP in the resolver's preference order: port 4070 first,
falling back to 443 and 80. Only outbound connections are needed; no
inbound ports have to be open.
