---
title: What is Fastpotify?
description: Why Fastpotify exists, what it does, and what it honestly does not do.
nav_order: 0
---

## The problem

The official Spotify client is a website in a box: a bundled browser engine
that takes the better part of a gigabyte of memory to show you a list of
songs. On Linux it is an afterthought; on a small laptop it is a tax. The
lightweight alternatives mostly live in the terminal, which is a fine place
to live, but not everyone wants their music there.

Fastpotify is a native Spotify client written in Rust with
[egui](https://github.com/emilk/egui), playing music through
[librespot](https://github.com/librespot-org/librespot). One small binary,
no browser engine anywhere in the process, a launch measured in fractions of
a second, with the layout you already know from Spotify, so there is almost
nothing new to learn.

![Fastpotify showing a playlist with the queue open and a track playing](/screenshot.png)

## What it does

- **Plays music on this computer.** Fastpotify is a Spotify Connect device:
  pick it from your phone, or press play here. Gapless, up to 320 kbps, with
  optional volume normalisation and an on-disk audio cache.
- **Controls every other device.** Move playback to a speaker, a phone, or
  another computer from the device picker, and keep controlling it: play,
  pause, skip, seek, shuffle, repeat, volume.
- **Your whole library.** Playlists, Liked Songs, saved albums, followed
  artists, podcasts, and saved episodes, filterable in the sidebar and as
  full pages. Playlists you own can be created, edited, and reordered.
- **Search** across songs, artists, albums, playlists, podcasts, and
  episodes, with artist pages, discographies, and related artists.
- **Stays out of the way.** Closing the window keeps the music playing from
  the system tray. MPRIS means your media keys, your bar, and `playerctl`
  see it like any other player. Every common action has a keyboard shortcut.
- **Looks the part.** Pages and the player bar take a tint from the album
  art of whatever you are looking at or listening to; light and dark themes,
  or follow the system.

## What it does not do yet

This is a young project, and it says so:

- **Spotify Connect on this computer needs Spotify Premium**, as with every
  librespot-based client. Browsing, search, and remote control work on any
  account. Free or unconfirmed accounts use alternate local audio for
  third-party matches; those matches are not Spotify audio.
- Sign-in happens twice in a lifetime, not once: the Web API and streaming
  are separate grants at Spotify. [How it connects](/how-it-connects/)
  explains why.
- No lyrics, no video podcasts, no social features.
- Playlist reordering is a menu action, not drag-and-drop.
- Fastpotify is an **unofficial** client built on Spotify's public Web API
  and librespot. Spotify changes these from time to time; when they do,
  features can break until the client catches up.

If something misbehaves, [an issue](https://github.com/crmne/fastpotify/issues)
with the terminal output of `fastpotify -v` and what you expected instead is
gold.

## Prior art

Fastpotify stands on earlier efforts:
[librespot](https://github.com/librespot-org/librespot) reimplemented
Spotify's playback protocol and carries every open client, including this
one; [spotify-tui](https://github.com/Rigellute/spotify-tui),
[spotify-player](https://github.com/aome510/spotify-player), and
[ncspot](https://github.com/hrkfdn/ncspot) proved how much client fits in a
few megabytes; and [Omarchy Spotify](https://github.com/stappmus/Omarchy-Spotify)
showed what a full-featured lightweight Spotify experience can look like on
the Linux desktop.

Fastpotify is an independent project, not affiliated with or endorsed by
Spotify AB. Spotify is a trademark of Spotify AB.
