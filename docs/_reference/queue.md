---
title: The Queue's Rules
description: What the queue shows, what Play next does, and what the app promises about both.
nav_order: 2
---

The queue is the list of what plays next. It has two parts. On top,
under **Playing next**, are the songs you queued yourself. Below them,
under **Next up**, are the songs that come next in whatever playlist or
album is playing. Your songs always play first.

These are the rules the app follows. The queue tests in `src/app.rs`
check every one of them.

1. **The list shows the play order.** The top row plays next, followed by the
   rows below it.

2. **Play next adds a song to your part of the queue.** It goes after
   the songs you queued earlier and before the playlist's songs. Queue
   the same song twice and it plays twice. A double-click only counts
   once.

3. **When a song starts, its row leaves the queue.** It doesn't matter
   how it started: the song before it ended, you pressed Next, you
   clicked it, or another device skipped to it. A song is never shown
   as playing and as next at the same time.

4. **Next removes the top row right away.** The app doesn't wait for
   Spotify to confirm it.

5. **Playing a row from the queue skips to it.** The rows above it are
   skipped and removed, as if you had pressed Next down to it. The rows
   below it stay, and the playlist keeps going afterwards.

6. **Starting a new playlist keeps your songs.** The rows underneath
   change to the new playlist; your songs stay on top and still play
   first.

7. **Clear only removes your songs.** The trash button sits beside the
   *Playing next* heading and empties that section; the playlist's rows
   below stay. It only shows while this computer is the player, because
   that is the only queue the app can actually clear.

8. **Changes appear immediately.** Fastpotify updates the queue before Spotify
   confirms the change. For local playback, it updates its own player directly.

9. **Closing the app keeps the queue.** Fastpotify saves it locally. When you
   resume the last song, it restores your queued songs and playlist position.

10. **Old answers from Spotify are ignored.** Queue responses can be a few
    seconds late. Fastpotify ignores stale responses and asks again. Your
    changes stay visible while it waits for confirmation.
