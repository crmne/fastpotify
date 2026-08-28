---
title: Getting Started
description: Install Fastpotify, sign in through your browser, and enable playback on this computer.
nav_order: 2
---

## Install

The [Download page](/download/) has the right file for every OS: a
drag-to-Applications app for macOS, zips for Windows, archives for Linux.

Or build from source with [Rust](https://rustup.rs) 1.95 or newer:

```sh
git clone https://github.com/crmne/fastpotify
cd fastpotify
cargo install --path .
```

On Linux the GUI needs the development packages any egui application does,
plus audio. On Arch:

```sh
sudo pacman -S --needed alsa-lib libpulse libxkbcommon wayland
```

On Debian or Ubuntu:

```sh
sudo apt install libasound2-dev libpulse-dev libxkbcommon-dev libwayland-dev libgl1-mesa-dev
```

Titles in a script the interface font does not cover -- Chinese, Japanese,
Korean, Arabic, Hebrew, Thai, the Indic scripts and a dozen more -- are drawn
with a face found on the system; Fastpotify bundles none. macOS and Windows
already carry faces for the common ones, and on Linux `noto-fonts` and
`noto-fonts-cjk` (Arch) or `fonts-noto` and `fonts-noto-cjk` (Debian or
Ubuntu) turn empty boxes back into characters.

![Japanese, Chinese, and Korean titles in a playlist](/assets/images/scripts.png)

A desktop entry ships in `packaging/applications/fastpotify.desktop`.

## Sign in

Start the app and press **Sign in with Spotify**. Your browser opens
Spotify's own consent page; your password never touches Fastpotify. When
Spotify redirects back, your library loads and you can search, browse, and
control your other devices immediately.

The sign-in is stored as a refresh token in your platform's state directory
(`~/.local/state/fastpotify` on Linux), so the browser is needed once per
machine. The next launch goes straight to your library.

## Enable playback on this computer

Playing music *on this machine* is one more one-time browser approval,
because Spotify treats streaming as a separate grant
([why](/how-it-connects/)). Take it from the device menu (the speaker icon
in the player bar, then **Play here, set up once**) or from Settings.
It needs Spotify Premium, and it too is remembered forever.

After that, this computer shows up as a Spotify Connect device named
**Fastpotify** (rename it in Settings), visible from your phone like any
speaker.

## Alternate local audio (optional)

Spotify Connect on this computer is the default and needs Premium. Settings
also has **Alternate local audio**. Fastpotify selects it for a Free account,
or until Spotify confirms Premium; Premium users can select it in Settings.
The app still uses the Spotify Web API for your library and search, then looks
up a third-party match (a Piped API you point at, and/or yt-dlp) and plays
that audio locally.

That mode is not Spotify Connect, not Spotify audio, and not a way to bypass
DRM. Fastpotify does not ship a Piped instance. It does embed an official
pinned yt-dlp build in each supported release binary, extracts it into the
local state directory, and never downloads yt-dlp at runtime. A yt-dlp you
installed is used only when its version is strictly newer than that pin.
You are responsible for the Piped endpoint and any yt-dlp you run, and for
their terms. Podcasts are not supported. A weak match is never played; you
can choose to skip to the next track instead. Playback starts after a short
buffer. An M4A file with metadata at the end may wait until download
finishes. Network stalls and transient HTTP errors retry and resume from
received ranges. A terminal transport or decode failure stops instead of
skipping.

## A few things worth knowing on day one

- **Closing the window does not stop the music.** Fastpotify keeps playing
  from the system tray; reopen it from the tray icon and quit from the tray
  menu or Ctrl+Q. Settings can turn this off.
- **Play buttons tell you what is happening.** A pressed play button spins
  until Spotify reacts, so the app is never silently "stuck".
- **The keyboard does everything.** Space plays and pauses, Ctrl+F or `/`
  searches, `Q` opens the queue; Ctrl+/ lists all of it.
- **Right-click is everywhere.** Every song, playlist, album, and artist has
  a context menu: queue it, save it, add it to a playlist, copy a link.
