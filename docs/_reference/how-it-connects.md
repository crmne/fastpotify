---
title: How It Connects
description: The two Spotify grants, why they are separate, what is stored, and what the client does when Spotify pushes back.
nav_order: 1
---

## Two grants, once each

Fastpotify talks to Spotify in two distinct ways, and Spotify issues
credentials for them separately:

1. **The Web API** covers your library, search, playlists, and devices. Fastpotify
   uses the standard Authorization Code + PKCE flow in your browser, as a
   registered Spotify application. The refresh token is stored locally and
   renewed automatically; your password never touches the app.
2. **Streaming** is actually playing audio on this computer, through
   [librespot](https://github.com/librespot-org/librespot). This runs the
   same browser flow once against Spotify's streaming client identity, after
   which librespot stores its own reusable credential. Premium is required,
   because that is what Spotify's streaming protocol requires.

Why not one grant? Because Spotify throttles Web API calls made with
streaming-identity tokens. Measured during development, every endpoint
answers `429` within the first request. Two narrow grants are what actually
works, and each one happens exactly once per machine.

By default the Web API uses the shared public application also used by
spotify-player, ncspot, and Omarchy Spotify, whose allowance Spotify
divides among everyone running any of them. An application of your own
gets one to itself; [Make It Even Faster](/make-it-even-faster/) shows
how, in five minutes.

## What the client stores

- The Web API refresh token and librespot's reusable credential, owner-only,
  in the state directory ([where](/settings-and-files/)).
- Downloaded audio and artwork, in the cache directory, within the budget
  you set.
- Lyrics, in the cache directory, for a month.
- Nothing else. There is no telemetry, no analytics, and no server of ours.
  Besides Spotify (and its album art CDN), the app talks to
  [lrclib.net](https://lrclib.net) while the lyrics panel is open and
  Spotify itself has no words for the track, sending the track's artist,
  title, album, and length, and to
  api.github.com once a day to learn whether a newer release exists, which
  Settings can turn off. Turning on the lyrics panel's **Translate** or
  **Romanize** also sends the song's words to Google Translate, which
  returns them echoed in your language or spelled in Latin letters; this
  is off by default, and the answers join the lyrics in the cache.

## When Spotify pushes back

The Web API rate-limits bursts. Fastpotify bounds its concurrency, honours
`Retry-After`, retries quietly, and shows a small spinner in the top bar
when a conversation with Spotify takes longer than a moment. Spotify also
reshapes endpoints over time; the client detects several of these shapes at
runtime and falls back to the older form where one still exists.

## Receivers on the local network

Spotify's device list contains only receivers already logged in to the
account. Anything waiting to be given one, which is the normal state for a
self-hosted librespot or spotifyd, is invisible to the Web API.

Those receivers announce themselves over mDNS as `_spotify-connect._tcp` and
answer a small HTTP interface. Fastpotify asks a receiver to describe itself,
then hands over the reusable credential librespot already stores, wrapped
twice: once in a key derived from the receiver's own device id, and again in
a key both sides derive from a Diffie-Hellman exchange with the public key
that receiver just published. A blob captured from the network is useless to
anything else, and the credential is never written anywhere new.

The receiver logs in with it, appears in the ordinary device list, and
everything after that is the plain Web API.

## The engine

Playback runs on a dedicated runtime: librespot maintains the Spotify
Connect session, so this computer appears as a device to every other Spotify
client you own, receives transfers, and reports its position back. If the
session drops, the engine reconnects with the stored credential; the
interface never blocks on any of it.
