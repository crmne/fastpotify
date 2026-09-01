---
title: Getting Started
description: Install Fastpotify, sign in through your browser, and enable playback on this computer.
nav_order: 2
---

## Install

The [Download page](/download/) has installers and archives for macOS,
Windows, and Linux.

Or build from source with [Rust](https://rustup.rs) 1.95 or newer:

```sh
git clone https://github.com/crmne/fastpotify
cd fastpotify
cargo install --path .
```

On Linux, install the GUI and audio development packages. On Arch:

```sh
sudo pacman -S --needed alsa-lib libpulse libxkbcommon wayland
```

On Debian or Ubuntu:

```sh
sudo apt install libasound2-dev libpulse-dev libxkbcommon-dev libwayland-dev libgl1-mesa-dev
```

Fastpotify uses system fonts for scripts that its interface font does not
cover, including Chinese, Japanese, Korean, Arabic, Hebrew, Thai, and Indic
scripts. macOS and Windows include fonts for the common cases. On Linux,
install `noto-fonts` and `noto-fonts-cjk` (Arch) or `fonts-noto` and
`fonts-noto-cjk` (Debian or Ubuntu) if titles appear as empty boxes.

![Japanese, Chinese, and Korean titles in a playlist](/assets/images/scripts.png)

A desktop entry ships in `packaging/applications/fastpotify.desktop`.

## Sign in

Start the app and press **Sign in with Spotify**. Your browser opens Spotify's
consent page, so Fastpotify never sees your password. When the browser returns
to the app, your library loads.

Fastpotify stores a refresh token in your platform's state directory
(`~/.local/state/fastpotify` on Linux). You normally need the browser only
once per machine.

## Enable playback on this computer

Playing music *on this machine* needs a second browser approval because
Spotify authorizes streaming separately ([why](/how-it-connects/)). Open the
device menu in the player bar and select **Set up playback here**, or use
Settings. This needs Spotify Premium. Fastpotify saves the playback credential.

The computer then appears as a Spotify Connect device named **Fastpotify**.
You can rename it in Settings.

## Basics

- **Closing the window does not stop the music.** Fastpotify keeps playing
  from the system tray; reopen it from the tray icon and quit from the tray
  menu or Ctrl+Q. On macOS you can also reopen it from the Dock. Settings can
  turn this off.
- **Play buttons show progress.** The button spins until Spotify responds.
- **Common actions have shortcuts.** Space plays and pauses, Ctrl+F or `/`
  searches, and `Q` opens the queue. Ctrl+/ shows the full list.
- **Rows and cards have context menus.** Right-click a song, playlist, album,
  or artist to see actions such as queue, save, add to playlist, and copy link.
