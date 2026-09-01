---
title: The Winamp Mini Player
description: Use classic Winamp 2 skins with an analyser, equalizer, and playlist.
nav_order: 4
---

Open the mini player with Ctrl+M (Cmd+Shift+M on macOS), the shrink button, or
**Switch to it** in Settings. It supports classic Winamp 2 `.wsz` skins. Find
skins at the [Winamp Skin Museum](https://skins.webamp.org).

Only one player window is open at a time. Click the skin logo or Eject, or use
the shortcut again, to return to the main window.

![The mini player wearing the built-in skin](/assets/images/winamp.png)

## Skins and window size

Drop a `.wsz` file on either window to install and use it. Settings lists the
installed skins and can open the skins folder.

The mini player uses whole-number scaling to keep pixels sharp. Right-click
the title bar, or click **O**, to choose 1x to 4x and set always-on-top. **D**
toggles double size and **A** toggles always-on-top. Fastpotify remembers the
window position.

Non-rectangular skins use `region.txt` for transparent areas. Winamp 3 and 5
skins use a different format and are not supported.

## Main controls

Most controls match Winamp. These work differently:

- **Stop** pauses and rewinds.
- **I** opens the playing album in the main window.
- Repeat is either on or off.
- The X button and both logos return to the main window.

Click the time to switch between elapsed and remaining time. The balance and
MONO/STEREO controls affect playback on this computer. Quit from the
right-click menu or with Ctrl+Q.

The shade button, or a double-click on the title bar, rolls the player up. The
playlist and equalizer have their own shade buttons.

The left display shows the spectrum analyser. Click it to switch to the
oscilloscope, then off. You can also use the **V** menu. The visualiser uses
local audio after the equalizer and before volume, so it still moves at zero
volume. It stays flat when another device is playing.

## Playlist

**PL** opens the playlist below the player. It shows the playing song followed
by the queue. Double-click a song to play it, Ctrl-click to select several, and
drag the lower-right corner to resize the window. Use X or **PL** to close it.

- **ADD** opens search or Liked Songs.
- **SEL** selects rows.
- **MISC** opens song, artist, and album pages.
- **LIST OPTS** starts one of your playlists or saves the queue as a new one.
- **REM → Remove all** clears your queued songs when this computer is playing.

Spotify does not let third-party apps remove one song from the queue. Notices
from the main window scroll through the mini player's text display.

## Equalizer

**EQ** opens the ten-band equalizer. It affects playback on this computer, not
other Spotify Connect devices. The preamp ranges from -12 to 12 dB. **AUTO**
resets all bands. The same controls and presets are in Settings.
