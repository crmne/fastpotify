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

Music played on this computer follows the system's default output: plug in
headphones, connect a Bluetooth speaker, or pick another device in the sound
settings, and playback moves there within a couple of seconds.

## Home

Home previews your most-played songs. Select **Your top songs** or **Show
more top songs** to open the complete ranked list.

Track tables sort by their column headings: click **Title**, **Album**,
**Date added**, or the clock to sort by it, again to reverse, and a third
time to return to the list's own order.

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

The queue lives behind the list icon, as a side panel or a full page. Add
anything to it from a row's context menu.

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

Ctrl+M (Cmd+Shift+M on macOS), the shrink button beside the settings gear,
or **Switch to it** in Settings turns Fastpotify into a small player that
wears classic Winamp skins: the `.wsz` files of the Winamp 2 era, of which
the [Winamp Skin Museum](https://skins.webamp.org) keeps tens of thousands.
There is one window at a time; the logo in the skin's corner, Eject, or
Ctrl+M again brings the big window back where it was.

![The mini player wearing the built-in skin](/assets/images/winamp.png)

Drop a `.wsz` on either window and Fastpotify copies it into its skins
folder and puts it on. Settings lists every skin in that folder, with the
built-in one first, and has a button to open the folder.

The window is drawn at a whole number of screen pixels per skin pixel, so
the pixels stay crisp at any size. Pick 1x to 4x from the menu behind a
right-click on the title bar (or the **O** at the display's edge), where
always-on-top lives too; **D** toggles double size and **A** always on top,
as they did. Drag the title bar to move it; it reopens where you left it,
and the keyboard shortcuts work there too.

The buttons do what they say, with a few translations. **Stop** pauses and
rewinds, **I** opens the playing album in the big window, and repeat is on
or off. **PL** opens the playlist window under the player, in the skin's
own frame and colours and the small unsmoothed lettering of the time: what
is playing, then the queue; double-click a
song to play from there, Ctrl-click to select several, drag the corner to
make it taller, and its X or PL again closes it. Its buttons do what
Spotify allows of what Winamp's did: ADD finds music, SEL picks rows,
MISC opens the song's pages, LIST OPTS plays one of your playlists or
saves the queue as a new one, and REM only explains that no app can take
from Spotify's queue. Notices that the big window shows as toasts scroll
through the marquee here. **EQ** opens the equalizer between the player and the playlist: Winamp's
ten bands and its presets, shaping the music played on this computer (a
speaker across the room plays what Spotify sends it). The preamp only
turns down, and AUTO, which loaded a preset per song, stays off. The same
equalizer is in Settings with its curve drawn out. The X and both logos
of the main window bring back the big window; its shade button, or a
double-click on the title bar, rolls it up to a bar with the time, a small
transport, and a seek bar, as Winamp's shade mode did. Skins that are not
rectangles keep their shape: whatever their `region.txt` leaves out is
see-through. Quitting is in the right-click menu and Ctrl+Q. Fastpotify has no balance
control, so that slider is drawn but does nothing.
Click the time to count down instead of up. The balance slider moves the
sound between the speakers and the MONO and STEREO lamps are a switch,
both for music played on this computer. The playlist's own shade button
rolls it up to a title bar and down again, and the equalizer's rolls it up
to a bar that keeps the volume and balance on it as tiny sliders, as Winamp
2.9 drew it; skins from before then wear the built-in bar for that. The display's left box is the
spectrum analyser, peaks and all, in the skin's own colours; click it, or
**V**, for the oscilloscope, and again for nothing. It shows the sound
leaving this computer, so a device across the room leaves it flat. Modern
(Winamp 3 and 5) skins are a different format and are not supported.

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
| `Ctrl+↑` / `Ctrl+↓` | Volume |
| `M` | Mute |
| `S` / `R` | Shuffle / cycle repeat |
| `Q` | Queue panel |
| `Ctrl+F` or `/` | Search |
| `Ctrl+B` | Show or hide the sidebar |
| `Alt+←` / `Alt+→` | Back or forward |
| `Ctrl+H` / `Ctrl+L` | Home / Liked Songs |
| `Ctrl+Shift+A` / `Ctrl+Shift+B` | Playing artist / album |
| `Ctrl+M` | Winamp mini player |
| `Ctrl+,` | Settings |
| `Ctrl+/` or `?` | All shortcuts |
| `Ctrl+Q` | Quit |

On macOS, `Cmd` replaces `Ctrl`.

## Settings

Settings (Ctrl+,) includes the Connect device name, audio quality up to
320 kbps, volume normalisation, autoplay, gapless playback, the audio backend
on Linux, the audio cache size, the equalizer, themes, album-art tinting,
compact views of the sidebar and of track lists (Spotify's compact views:
names only, one line a row, no covers),
the mini player's skin and size, and close-to-tray behaviour. Applying playback settings restarts the local player. Other
settings take effect immediately.
