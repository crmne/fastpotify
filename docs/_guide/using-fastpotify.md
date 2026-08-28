---
title: Everyday Use
description: Playing music, managing your library and playlists, devices, the queue, the tray, and every keyboard shortcut.
nav_order: 3
---

## Playing music

Play buttons are wherever you expect them: on playlist, album, artist, and
podcast pages, on cards when you hover them, and on every row. Double-click
a row to play from that song within its playlist or album. The shuffle
button next to a page's play button starts the whole thing shuffled.

The player bar at the bottom always shows what is playing, on this
computer *or* on any other device. Click the title to open its album, the
artist to open the artist, the heart to save it.

## Home

Home previews your most-played songs. Select **Your top songs** or **Show
more top songs** to open the complete ranked list.

Track tables sort by their column headings: click **Title**, **Album**,
**Date added**, or the clock to sort by it, again to reverse, and a third
time to return to the list's own order.

## Your Library

The sidebar keeps the playlists you played most recently on top,
wherever you played them, and remembers that order between runs.

The sidebar is your library: filter it by Playlists, Albums, Artists, or
Podcasts with the chips, or search it with the magnifier. Liked Songs is
pinned on top. The current page is highlighted; the playlist that is playing
carries a small speaker.

**Playlists you own** are fully editable: create one with the **+** button,
add songs from any row's menu, remove and reorder from the playlist page,
and rename or delete from its context menu. Playlists you follow can be
followed and unfollowed.

## Search

Ctrl+F (or `/`) focuses search from anywhere. Results come grouped (a top
result, songs, artists, albums, playlists, podcasts, episodes) and the
chips narrow to one kind. Your recent searches wait on the empty search
page.

## Devices and the queue

The speaker icon in the player bar lists every Spotify Connect device on
your account. Click one and the music moves there mid-song; the same
controls keep working. "Playing on …" in the top bar reminds you when sound
is coming out of something across the room.

The queue lives behind the list icon, as a side panel or a full page. Add
anything to it from a row's context menu.

### Speakers Spotify has not noticed yet

A receiver running librespot or spotifyd, and some hardware speakers, only
appear in Spotify's own device list once an account has been handed to them.
Until then the Web API cannot see them at all, however plainly they show up
in the official client.

Fastpotify looks for those on your local network whenever you open the
device picker, and lists them under the devices Spotify already knows about,
marked *on your network*. Choose one and Fastpotify hands it your account
over the local network, encrypted so that only that receiver can read it.
A moment later it joins Spotify Connect properly and playback moves there,
and from then on it is an ordinary device to every Spotify client you own.

This uses the credential stored for playing on this computer, so enable
playback here first (see [Getting Started](/getting-started/)). Receivers
that ask for a different kind of login are not connected this way yet.

## Lyrics

The microphone button in the player bar (or `L`) opens the words of the
playing track beside the page. When the lyrics are timed, the line being
sung is highlighted and the panel follows along; click any line to jump the
song there. Scrolling by hand stops the following, and **Follow** in the
panel's header picks the song back up. When this computer is signed in
for playback, the words come from Spotify itself. Otherwise, and for
tracks Spotify has no words for, they come from
[LRCLIB](https://lrclib.net), an open database that needs no account, so
they work for whatever is playing, on this computer or another device;
podcasts and tracks nobody has transcribed say so.

![The lyrics panel beside a playlist, following the song](/assets/images/lyrics.png)

## The tray

Closing the window keeps the music playing: Fastpotify stays in the system
tray with play, pause, skip, and quit in its menu, and clicking the icon
brings the window back. On Linux it is a standard status-notifier item, so
it works in any bar that shows tray icons, and MPRIS keeps `playerctl`,
media keys, and your desktop's players widget working the whole time.

## One window, one instance

Starting Fastpotify while it is already running does not open a second copy.
The launch hands the request to the instance already there, which brings its
window forward, and then gets out of the way. Two copies would mean two
Spotify Connect devices with the same name and two players arguing over your
media keys, so there is only ever one.

## Keyboard shortcuts

| Shortcut | What it does |
| --- | --- |
| `Space` | Play or pause |
| `Ctrl+←` / `Ctrl+→` | Previous or next |
| `Shift+←` / `Shift+→` | Seek 10 seconds |
| `Ctrl+↑` / `Ctrl+↓` | Volume |
| `M` | Mute |
| `S` / `R` | Shuffle / cycle repeat |
| `Q` | Queue panel |
| `Ctrl+F` or `/` | Search |
| `Ctrl+B` | Show or hide the sidebar |
| `Alt+←` / `Alt+→` | Back or forward |
| `Ctrl+H` / `Ctrl+L` | Home / Liked Songs |
| `Ctrl+Shift+A` / `Ctrl+Shift+B` | Playing artist / album |
| `Ctrl+,` | Settings |
| `Ctrl+/` | All shortcuts |
| `Ctrl+Q` | Quit |

On macOS, `Cmd` replaces `Ctrl`.

## Settings

Everything lives in Settings (Ctrl+,): the Connect device name, audio
quality up to 320 kbps, volume normalisation, autoplay, gapless playback,
the audio backend on Linux, the audio cache size, themes, album-art
tinting, and the close-to-tray behaviour. Playback settings apply with one
button that restarts the local player; nothing else needs a restart.
