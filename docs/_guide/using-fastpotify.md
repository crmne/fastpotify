---
title: Everyday Use
description: Playing music, managing your library and playlists, devices, the queue, the tray, and every keyboard shortcut.
nav_order: 3
---

## Playing music

Playlist, album, artist, and podcast pages have play buttons. Cards show one
when you hover, and each row has its own. Double-click a row to start playback
from that song within its playlist or album. The shuffle button next to a
page's play button starts the page in shuffled order.

The player bar shows what is playing locally or on another device. Click the
title to open its album, the artist name to open the artist, or the heart to
save the track.

When you start Fastpotify and nothing is playing anywhere, the player bar
shows the song the last session ended on, paused at the second it stopped at.
Press play and it carries on from there rather than starting the song again.
The rest of the transport works from that standstill too: next and previous
step through the playlist it was left in without starting anything, previous
restarts the song when it is more than a few seconds in, and dragging the
progress bar moves where play will resume from.

Music played on this computer follows the system's default output: plug in
headphones, connect a Bluetooth speaker, or pick another device in the sound
settings, and playback moves there within a couple of seconds.

## Home

Home previews your most-played songs. Select **Your top songs** or **Show
more top songs** to open the complete ranked list.

Track tables sort by their column headings: click **Title**, **Album**,
**Date added**, or the clock to sort by it, again to reverse, and a third
time to return to the list's own order. Recent additions say how long ago
they were added; entries at least a month old show their calendar date.

![A playlist showing relative dates for recent additions](/assets/images/date-added-relative-demo.png)

## Your Library

Pinned entries sit in a block right under Liked Songs: pin one from its
context menu, drag a row into the block to pin it where you drop it,
drag within the block to reorder it, and drag a pinned row below the
block to unpin it.

Below the pins, the sidebar starts out sorting playlists by when you
last played them. Drag one to a new place and the rest of the shelf
switches to your own order instead: rows stay exactly where you drop
them, and new playlists wait just under the pins until you place them.
Choose **Sort by recently played** from any playlist's context menu to
go back; dragging a row switches to your own order again.

The Albums, Artists, and Podcasts shelves pin the same way: drag into
the block, within it, or below it.

Use the chips to filter the sidebar by Playlists, Albums, Artists, or Podcasts,
or use the magnifier to search it. Liked Songs stays at the top. The current
page is highlighted, and the playing playlist has a small speaker icon.

**Playlists you own** are fully editable: create one with the **+** button,
add songs from any row's menu or by dragging them onto a playlist in the
sidebar, remove and reorder from the playlist page, and rename or delete
from its context menu. Reordering works by dragging a row to its new
place, or from its menu; while the table is sorted or filtered, rows
keep their place. Dropping a song on Liked Songs saves it. Playlists
you follow can be followed and unfollowed.

## Search

Ctrl+F (or `/`) focuses search from anywhere. Results are grouped into top
result, songs, artists, albums, playlists, podcasts, and episodes. Use the
chips to show one type. The empty search page lists recent searches.

## Devices and the queue

The speaker icon in the player bar lists every Spotify Connect device on
your account. Click one and the music moves there mid-song; the same
controls keep working. "Playing on …" in the top bar reminds you when sound
is coming out of something across the room.

The queue lives behind the list icon, as a side panel or a full page.
*Play next* in a row's context menu queues a song. Songs you queued sit
on top under *Playing next* and play first; the playlist or album you
were listening to carries on underneath, under *Next up*. Point at a
song and press the play button on its cover (or double-click it) to
jump straight to it, and the trash icon beside *Playing next* clears
what you queued while the playlist carries on. Clearing works when this
computer is the player; Spotify offers no way to empty another device's
queue from afar. Closing the app keeps the queue; it is back as you left it on
the next start. The list-plus icon saves the queue as a new playlist of
yours, each song once, in playing order; a song radio you like becomes
a playlist named after its song. *Go to song radio* in a song's context
menu starts Spotify's station for it and opens the queue, which is
where the station lives. The full set of rules is in
[The Queue's Rules](/queue/).

### Receivers on the local network

A receiver running librespot or spotifyd, and some hardware speakers, appears
in Spotify's device list only after it has received an account credential.
Before then, the Web API cannot see it.

Fastpotify searches the local network when you open the device picker. It
lists discovered receivers as *on your network*. Choose one to send it the
stored playback credential, encrypted so that only that receiver can read it.
Once connected, it appears as an ordinary Spotify Connect device and playback
moves to it.

This uses the credential stored for playing on this computer, so enable
playback here first (see [Getting Started](/getting-started/)). Receivers
that ask for a different kind of login are not connected this way yet.

## Lyrics

The microphone button in the player bar (or `L`) opens lyrics for the playing
track beside the page. For timed lyrics, the current line is
highlighted and the panel scrolls automatically; click a line to seek to it.
Manual scrolling pauses automatic following, and **Follow** resumes it.
Fastpotify requests lyrics from Spotify when local playback is authorized.
Otherwise, or when Spotify has no lyrics for a track, it uses
[LRCLIB](https://lrclib.net), an open database that needs no account. Podcasts
and tracks without a transcription show an unavailable message.

![The lyrics panel beside a playlist, following the song](/assets/images/lyrics.png)

## The Winamp mini player

Ctrl+M (Cmd+Shift+M on macOS) turns Fastpotify into a small player that
wears classic Winamp skins. It has [a page of its own](/winamp/).

## The tray

Closing the window keeps the music playing: Fastpotify stays in the system
tray with play, pause, skip, and quit in its menu, and clicking the icon
brings the window back. On Linux it is a standard status-notifier item, so
it works in any bar that shows tray icons, and MPRIS keeps `playerctl`,
media keys, and your desktop's players widget working the whole time.

## One window, one instance

Starting Fastpotify while it is already running brings the existing window
forward instead of opening a second instance. This avoids duplicate Spotify
Connect devices and conflicting media-key handlers.

## Keyboard shortcuts

| Shortcut | What it does |
| --- | --- |
| `Space` | Play or pause |
| `Ctrl+←` / `Ctrl+→` | Previous or next |
| `Shift+←` / `Shift+→` | Seek 10 seconds |
| `Ctrl+↑` / `Ctrl+↓`, or the wheel over the volume slider | Volume |
| `M` | Mute |
| `S` / `R` | Shuffle / cycle repeat |
| `Q` | Queue panel |
| `Ctrl+F` or `/` | Search |
| `Ctrl+B` | Show or hide the sidebar |
| `Alt+←` / `Alt+→`, or the mouse's back and forward buttons | Back or forward |
| `Ctrl+H` / `Ctrl+L` | Home / Liked Songs |
| `Ctrl+Shift+A` / `Ctrl+Shift+B` | Playing artist / album |
| `Ctrl+M` | Winamp mini player |
| `Ctrl+W` | Close the window (the tray keeps playing, if that is on) |
| `Ctrl+,` | Settings |
| `Ctrl+/` or `?` | All shortcuts |
| `Ctrl+Q` | Quit |

On macOS, `Cmd` replaces `Ctrl`.

## Settings

Settings (Ctrl+,) includes the Connect device name, audio quality up to
320 kbps, volume normalisation, autoplay, gapless playback, the audio backend
on Linux, the audio cache size, the equalizer, themes, album-art tinting,
autoscroll, compact views of the sidebar and of track lists (Spotify's
compact views: names only, one line a row, no covers),
the mini player's skin and size, and close-to-tray behaviour. Applying playback settings restarts the local player. Other
settings take effect immediately.

### Autoscroll

Click the middle mouse button in any scrollable list, then move the mouse —
the page follows, faster the further the cursor sits from where you
clicked. Move down to read further down, right to reveal the rest of a
wide row. A small dead zone around the click keeps a plain middle-click
from nudging the page. Click again (any button) or press Esc to stop. It
is on by default on Windows and macOS, and off by default on Linux, where
the middle button instead pastes selected text — turn it on there with the
**Autoscroll** switch in Settings, Appearance.
