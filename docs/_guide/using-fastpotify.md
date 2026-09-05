---
title: Everyday Use
description: Library ordering and local play history.
nav_order: 3
---

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

By default, the sidebar sorts playlists by when you last played them. Drag a
playlist to switch to a custom order. New playlists appear below the pinned
group. Choose **Sort by recently played** from a playlist's context menu to
restore the default order.

## Recent

The queue panel's second tab combines Spotify's history with tracks played
through Fastpotify, which Spotify does not record.

A song is added after about 30 seconds, or halfway through a shorter song.
Paused time and seeking do not count.

The local list is stored in `history.json` and is never uploaded. Settings →
Storage shows its location and has a **Clear history** button.

On Windows, the main window's minimize, maximize, and close buttons share the
top bar with Fastpotify's controls. Drag an empty part of that bar to move or
snap the window, and drag a window edge or corner to resize it.
