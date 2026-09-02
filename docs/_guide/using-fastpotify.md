---
title: Everyday Use
description: Library ordering, podcast speed, and local play history.
nav_order: 3
---

## Library order

By default, the sidebar sorts playlists by when you last played them. Drag a
playlist to switch to a custom order. New playlists appear below the pinned
group. Choose **Sort by recently played** from a playlist's context menu to
restore the default order.

## Podcast speed

A podcast episode playing on this computer shows a speed button next to the
transport controls. Click it to cycle through 0.5×, 0.8×, 1×, 1.2×, 1.5×, 2×,
3×, and 3.5×. The voice keeps its pitch, the progress bar keeps time, and the
choice is remembered for later episodes. Songs always play at normal speed,
and playback on another device runs at that device's own rate.

One limitation: while an episode plays here at anything but 1×, other Spotify
apps show its position falling behind. Spotify Connect only learns the
position when playback starts, pauses, seeks, or moves to the next episode,
and assumes normal speed in between. It catches up at the next of those, and
moving playback to another device mid-episode may pick up from that earlier
position. This window's own progress bar is always right.

![The player bar with a podcast episode playing at 1.5×](/assets/images/podcast-speed.png)

## Recent

The queue panel's second tab combines Spotify's history with tracks played
through Fastpotify, which Spotify does not record.

A song is added after about 30 seconds, or halfway through a shorter song.
Paused time and seeking do not count.

The local list is stored in `history.json` and is never uploaded. Settings →
Storage shows its location and has a **Clear history** button.
