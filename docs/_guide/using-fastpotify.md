---
title: Everyday Use
description: Library ordering and local play history.
nav_order: 3
---

## Scrolling shelves

Point at a horizontal shelf, such as Made for you or Recently played on
Home, and hold `Shift` while turning the mouse wheel. The shelf moves while
the surrounding page stays put. Release `Shift` to scroll the page normally.

## Dragging beyond the visible list

On `main`, for the release after 0.7.1, hold a dragged song near the top or
bottom of an editable playlist's visible area to scroll. Scrolling gets faster
closer to the edge and stops when you move away or release the mouse. This
lets you move a song from the end to the beginning without dropping it along
the way. Clear filters and sorting before reordering playlist songs.

On `main`, after 0.7.1, drag a song from the player bar, the queue, or another
list into an open editable playlist. The line between rows marks its insertion
position. Dropping below the last row appends; the blank area of an empty
playlist accepts its first song. The source song stays in its list or queue,
and playback continues unchanged. Dragging a row within the same playlist
still moves that row.

Clear any playlist filter or sort before placing songs between rows, so the
visible positions match Spotify's order. A duplicate confirmation keeps the
chosen position when you select **Add anyway**. Dragging near the top or bottom
of the playlist scrolls to positions beyond the visible rows.

The Library sidebar also scrolls near its edges when you drag a song toward a
playlist or reorder its entries. Only the list under the pointer scrolls.

## Keyboard and screen readers

The main window provides screen-reader names for playback controls, library
and song rows, menus, sliders, and settings switches. `Tab` and `Shift+Tab`
move keyboard focus, shown by an outline. `Enter` or `Space` activates the
focused control; on a song row, it plays that song. The row's **More** button
opens its menu from the keyboard too.

Left and right arrows adjust a focused volume slider by five percentage
points, or the seek slider by one percent of the song. Screen readers can
also read and set these sliders' values. `Ctrl+F` (`Cmd+F` on macOS) focuses
search. The playback shortcuts remain available; unmodified letter and
Space shortcuts yield to the focused control.

This is the first part of screen-reader support. Windows testing with NVDA
remains tracked in [#262](https://github.com/crmne/fastpotify/issues/262).
Winamp skins do not yet have equivalent accessibility coverage.

## Library order

On `main`, after 0.7.1, the menu below the Library filters selects an order
for each section. **Name** and **Recently played** are available throughout.
Albums and podcasts also offer **Recently added**, using their actual save
dates. Spotify does not supply equivalent dates for followed playlists or
artists, so those sections do not offer that choice. Entries with missing save
dates come last.

**Spotify custom order** follows the playlist sequence and folders supplied by
the existing local playback session. Until that order arrives, available
playlists stay visible. The last good tree is kept for the same signed-in
account. Fastpotify's local pins remain at the top, including pins from a closed
folder. Changing an order or dragging a row here does not change Spotify's
order or folders.

Drag playlists to choose **Local custom order**. New playlists appear below the
pinned group. Selecting **Name**, **Recently played** or **Spotify custom order**
keeps the saved arrangement, so selecting **Local custom order** restores it.
The playlist context menu's **Sort by recently played** also preserves it.

Upgrading keeps the previous default: a saved local playlist arrangement wins;
otherwise available Spotify folders keep their order, and a flat playlist list
uses recent plays. Other sections keep their supplied Library order until you
select a sort. Explicit sorts load the remaining pages of the selected section
in the background. A failed page stops that loading; choosing the order again
retries it.

Liked Songs starts pinned at the top. Drag it between pins to choose its
position, or below the pin block to unpin it and put it in **Local custom
order**. Other pins can sit above it. Its right-click menu also offers **Unpin**
and **Pin to top**; pinning adds it after your existing pins. The arrangement
survives restarting Fastpotify and switching sort choices.

When unpinned, Liked Songs follows **Name** or **Recently played** like the other
rows. In **Spotify custom order**, it appears after the playlists because it
has no place in Spotify's playlist tree. Returning to **Local custom order**
restores its saved position. Dragging a song onto Liked Songs still saves that
song, wherever the row sits.

## Windows taskbar controls

On `main`, after 0.7.1, hovering Fastpotify's taskbar button offers **Previous**,
**Play/Pause**, and **Next** beneath its window preview. They control the same
playing device as the player bar, update immediately, and are disabled when
there is no song or the device refuses controls. The icons follow the system
appearance and display scaling.

Closing to the tray removes the window and its preview. Reopening the main
window or switching to the Winamp window creates its controls again. Media
keys and the system's now-playing controls continue working while the window
is closed. These buttons add no Spotify requests beyond their playback actions.

For the Winamp mini player, turn off **Show in taskbar** under
**Settings > Winamp skins**, or **Show in taskbar** in its options menu.
The choice survives restarts. The mini player stays visible; the tray icon,
**Ctrl+M**, the skin logo, and launching Fastpotify again remain ways to reach
the app. Returning to the main window always restores its taskbar button.
Changing the option while the mini player is open replaces that window while
playback continues. This setting is available on Windows; it does not change
Linux panels or the macOS Dock.

## Recent

The queue panel's second tab combines Spotify's history with tracks played
through Fastpotify, which Spotify does not record.

A song is added after about 30 seconds, or halfway through a shorter song.
Paused time and seeking do not count.

The local list is stored in `history.json` and is never uploaded. Settings →
Storage shows its location and has a **Clear history** button.

## Lyrics

Choose the microphone button in the player bar, or press **L**, to open lyrics.
Synced lyrics follow the playing line automatically. Scroll to pause following,
choose **Follow** to resume it, or choose a line to seek there. The expand
button opens the full-screen view; press **Esc** or choose the shrink button to
return to the previous window mode. The waveform button reduces or restores
scroll and highlight motion.

On Windows, the main window's minimize, maximize, and close buttons share the
top bar with Fastpotify's controls. Drag an empty part of that bar to move or
snap the window, and drag a window edge or corner to resize it.
