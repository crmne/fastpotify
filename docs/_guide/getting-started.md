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

Chinese and Japanese titles are drawn with a font found on the system;
Fastpotify does not bundle one. macOS and Windows already have a suitable
face, and on Linux `noto-fonts-cjk` (Arch) or `fonts-noto-cjk` (Debian or
Ubuntu) turns empty boxes back into characters. Noto CJK covers Korean too,
which the faces macOS and Windows ship do not.

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
