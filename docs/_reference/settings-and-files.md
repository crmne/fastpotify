---
title: Settings & Files
description: Where Fastpotify keeps configuration, credentials, and caches, and what is safe to delete.
nav_order: 0
---

## Where things live

Fastpotify follows each platform's conventions. On Linux:

| What | Where | Safe to delete? |
| --- | --- | --- |
| Settings | `~/.config/fastpotify/settings.json` | Yes, you lose preferences |
| Winamp skins | `~/.config/fastpotify/skins/` | Yes, you add them again |
| MilkDrop presets | `~/.config/fastpotify/milkdrop/` | Yes, you fetch them again |
| Spotify grants (on `main`, after 0.7.1) | System credential store | Use Sign out in Settings |
| Credential revocation markers (on `main`, after 0.7.1) | `~/.local/state/fastpotify/credential-storage/` | Keep after a failed sign-out deletion |
| Legacy shared Web API grant | `~/.local/state/fastpotify/shared_web_api_token.json` | Removed after migration or sign-out |
| Legacy personal Web API grant | `~/.local/state/fastpotify/personal_web_api_token.json` | Removed after migration or sign-out |
| Legacy playback credential | `~/.local/state/fastpotify/credentials/` | Removed after migration or sign-out |
| Last session | `~/.local/state/fastpotify/session.json` | Yes |
| Play history | `~/.local/state/fastpotify/history.json` | Yes |
| Audio cache | `~/.cache/fastpotify/audio/` | Always |
| Artwork cache | `~/.cache/fastpotify/art/` | Always |
| Lyrics cache | `~/.cache/fastpotify/lyrics/` | Always |
| Account-scoped playlist page cache | `~/.cache/fastpotify/playlists/<account-id>/` | Always |
| Last run's log | `~/.local/state/fastpotify/fastpotify.log` | Always |
| Crash log | `~/.local/state/fastpotify/panic.log` | Always |

Clearing caches never signs you out. Sign-out from Settings covers the shared
and personal Web API grants and the independent playback credential.

The following credential storage is on `main`, for the release after 0.7.1.

Durable grants use **Secret Service on Linux**, **Keychain on macOS**, and
**Credential Manager on Windows**, under the service name
`rocks.fastpotify.Fastpotify`. Entries are separated by application state
location and grant type; Web grants carry their Client ID and must verify as
the same account. Playback and receiver activation require that account too.
Non-secret settings and session data remain readable JSON. Native credential
protection reduces exposure from copying ordinary application files. It does
not protect a usable session from arbitrary code running as the same user;
Linux protection also depends on the desktop keyring's configuration.

On Linux, enable and unlock a Secret Service provider such as GNOME Keyring or
KWallet to remember a new sign-in. Flatpak is allowed to talk to
`org.freedesktop.secrets` for this purpose. An unavailable or locked store
produces an error without blocking the interface. A new sign-in can still be
used for this session, with no new plaintext fallback file.

On upgrade, each legacy grant is written to the protected store and read back
before its old file is removed. Valid grants migrate without signing in again.
If Spotify rejects a saved refresh grant, only that grant is forgotten so the
next launch cannot keep restoring it. If migration fails, Fastpotify reports it and
keeps the original so migration can be retried. That grant can still serve the
current session. A successfully migrated grant is never replaced by a stale
legacy copy. Librespot's reusable grant stays in memory until Fastpotify saves
it through this same store. Volume and disposable audio caches are independent.

Sign-out invalidates pending authorization, refresh, and playback connections,
and cancels pending Spotify requests so their results cannot undo a new sign-in.
It records non-secret revocation markers before deleting the protected entries
and all legacy token files, including temporary copies. A locked store or
filesystem failure is reported. Revocation markers prevent a failed protected
entry deletion from restoring the session on restart; keep these markers when
a deletion failed. Removing or changing a personal Client ID clears that app's
old grant.

Version 0.7.1 and earlier use the legacy unencrypted files listed above. Their
Web API writer requests owner-only permissions for newly created Unix files;
Windows uses inherited permissions. Librespot's old writer uses the system's
file defaults. Keep these legacy files, their temporary copies, the
`credentials/` directory, and credential-store exports out of issue attachments
and diagnostic uploads.

Progress through a playlist is periodically cached as a contiguous prefix.
When the playlist has not changed on Spotify, reopening it resumes from that
prefix instead of requesting the same pages again. Fastpotify validates the
cache against Spotify's playlist snapshot before showing it.
Successful playlist edits keep that loaded prefix and save it under Spotify's
new snapshot. Fastpotify reloads the playlist only if the write fails and the
optimistic edit must be reconciled.

The following Liked Songs caching behavior is on `main`, for the release
after 0.7.1.

Liked Songs metadata is stored separately under `liked-songs/` in the cache
directory, one JSON file per account. Only the verified account's rows are
shown. Fresh cached pages are reused for 15 minutes; older pages refresh in
the background. Refreshing keeps the last usable rows until their replacement
is ready, and a failed refresh leaves those rows visible. The refresh control
requests current data immediately. Partial caches resume from their next page.
Like and Unlike change the rows immediately, and confirmed edits survive a
restart even if Spotify's next read still reports the old state. This cache
contains metadata, not offline audio, and can be deleted without signing out.

The last good playlist folder tree is kept in `session.json`, scoped to the
account that supplied it. This keeps folders visible when local playback is
temporarily unavailable. Live session data is still required for edit grants.

On `main`, for the release after 0.7.1, memory caches retain the open page,
the playing context, and a limited set of recently used playlist, album,
artist, and show pages. Older pages reload when revisited, using the saved
playlist prefix when its snapshot still matches. Pending playlist edits and
their rows stay in memory until the write and its snapshot are confirmed,
even if this temporarily exceeds the usual page limit. Track metadata is
limited to 800 cached tracks; navigation and periodic cleanup trim old entries.

The session remembers separate positions for the main window and the Winamp
mini player. The shade modes are kept in `settings.json`. Wayland compositors
may ignore saved positions. On Windows, a position
whose title bar is no longer on an available monitor's work area is discarded
when reopening the window, keeping its initial on-screen placement instead.

Large playlist pages also have a **Go to song** control. Entering a song
number loads its 50-item page directly, without requesting every earlier page.
Filtering or sorting still covers the whole playlist, so either action returns
to the beginning and loads the remaining pages as needed.

On `main`, for the release after 0.7.1, Flatpak also preserves the fallback
state directory used when `XDG_STATE_HOME` is unset. Session state, history,
logs, and credential revocation markers survive a full quit and relaunch under
`~/.var/app/rocks.fastpotify.Fastpotify/.local/state/fastpotify/`. Configuration
and caches remain under the app's `config/` and `cache/` directories. State
already lost on quitting an older release cannot be recovered.

On macOS, settings, state, and the logs are in
`~/Library/Application Support/me.paolino.fastpotify` and the caches in
`~/Library/Caches/me.paolino.fastpotify`. On Windows, settings are in
`%APPDATA%\paolino\fastpotify\config`, state and the logs in
`%LOCALAPPDATA%\paolino\fastpotify\data`, and the caches in
`%LOCALAPPDATA%\paolino\fastpotify\cache`.

## settings.json

Settings are stored in one readable JSON file and written atomically. Its
main fields are:

| Field | Default | Meaning |
| --- | --- | --- |
| `device_name` | `Fastpotify` | Name on Spotify Connect |
| `bitrate` | `320` | 96, 160, or 320 kbps |
| `normalisation` | `false` | Volume normalisation |
| `autoplay` | `true` | Keep playing similar music at the end |
| `gapless` | `true` | Gapless playback |
| `audio_backend` | platform | `pulseaudio` or `rodio` on Linux |
| `audio_cache_mb` | `1024` | On-disk audio cache budget |
| `theme` | `dark` | `dark`, `light`, or `system` |
| `accent_from_art` | `true` | Tint pages with album art |
| `library_sort` | `{}` | Per-section Library order overrides, after 0.7.1: `library`, `recently_played`, `name`, `recently_added`, `local`, or `spotify`, where supported |
| `sidebar_order` | `[]` | Saved local playlist arrangement, including an unpinned Liked Songs, retained when another sort is selected |
| `pinned_contexts` | `[]` | Local Library pin order; Liked Songs uses `fastpotify:liked-songs`, a local key never sent to Spotify |
| `liked_songs_pinned` | `true` | Keep Liked Songs in the pin block; older settings place it first until moved |
| `sidebar_compact` | `false` | Names only in the library sidebar, no covers |
| `tracklist_compact` | `false` | One-line track rows without covers |
| `winamp_window` | `false` | The window is the Winamp mini player |
| `winamp_show_taskbar` | `true` | Windows only, after 0.7.1: show the Winamp window's taskbar button; the main window always keeps its button |
| `skin` | none | File or folder name in the skins folder; blank uses the built-in skin |
| `skin_scale` | by display | Screen pixels per skin pixel, 1 to 4 |
| `winamp_on_top` | `false` | Keep the mini player above other windows |
| `vis` | `bars` | The mini player's visualiser: `bars`, `scope`, or `off` |
| `playlist_open` | `false` | The playlist window is open under the mini player |
| `playlist_height` | `174` | The playlist window's height in skin pixels |
| `eq_open` | `false` | The equalizer window is open under the mini player |
| `eq_on` | `false` | The equalizer shapes local playback |
| `eq_preamp_db` | `0` | The preamp, in decibels, -12 to 12 |
| `eq_bands_db` | ten zeros | The bands from 60 Hz to 16 kHz, in decibels, -12 to 12 |
| `balance` | `0` | Left to right, -1 to 1, for local playback |
| `mono` | `false` | Play both channels the same |
| `playlist_shaded` | `false` | The playlist window is rolled up to its title bar |
| `winamp_shaded` | `false` | The main window is rolled up to its title bar |
| `milkdrop_open` | `false` | The MilkDrop window is open |
| `milkdrop_seconds` | `30` | How long each MilkDrop preset plays |
| `milkdrop_fps` | `60` | MilkDrop frame rate; `0` is uncapped |
| `milkdrop_screen_hz` | `0` | Last reported display refresh rate |
| `milkdrop_fullscreen` | `false` | The MilkDrop window fills the screen |
| `milkdrop_size` | `640, 480` | The MilkDrop window's size in points |
| `keep_playing_in_background` | `true` | Close to tray |
| `check_for_updates` | `true` | Ask GitHub once a day for a newer release |
| `web_client_id` | none | Optional personal Spotify app id used alongside shared coverage |
| `personal_app_nudge_at` | none | Legacy daily-reminder timestamp, retained for older releases |
| `personal_app_intro_seen` | `false` | Whether the Premium personal-app introduction was dismissed or followed (on `main`, after 0.7.1) |

## Command line

```
fastpotify [OPTIONS] [LINK]

  LINK                  A Spotify link to open: spotify:track:…, or an
                        open.spotify.com address
  --device-name <NAME>  Spotify Connect name for this session
  -v, --verbose         More logs from librespot and the API client
```

A link goes to the running Fastpotify when there is one, which then opens
the page and brings its window forward; otherwise the app starts on it. The
desktop's handler for `spotify:` links runs exactly this.

Attach `fastpotify.log` from the state directory to bug reports. It contains
the last run's output, including extra lines from `fastpotify -v`. After a
crash, attach `panic.log` too.

## Demo mode

Builds made with `cargo build --features demo` accept `--demo`, which loads
sample data for screenshots and interface work. Demo mode never writes
settings.

`--demo-page` opens a page, such as `home`, `playlist:pl1`, or `artist:art0`,
and `--demo-show` adds surfaces on top of it: a comma separated list of
`queue`, `playing-next`, `devices`, `shortcuts`, `premium`, `create`, `duplicate`, `light`,
`focus`, `winamp`, `playlist`, `eq`, `eq-shade`, `compact`, `update`, and `personal-app`.
`update` shows a sample update badge for checking its layout.
`personal-app` shows the personal Spotify app introduction.

`--demo-shot <PATH>` writes the window to a PNG and exits, which is useful for
making deterministic screenshots for these pages:

```
cargo run --release --features demo -- \
  --demo-shot docs/screenshot.png --demo-page playlist:pl1 --demo-show queue
```

The image uses the current window size. `--demo-size WIDTHxHEIGHT` sets that
size for a shot (for example `760x800` or `1240x800`). `--demo-shot-delay <MS>`
sets how long to wait for cover art before taking it.

## Home layout

Quit Fastpotify before editing `settings.json`, then restart it. Add a `home`
object to hide shelves, limit their displayed items, or choose playlist sources:

```json
"home": {
  "quick_access": {
    "limit": 12,
    "liked_songs": true,
    "discover_weekly": true,
    "release_radar": true,
    "pinned_playlists": true,
    "library_playlists": false
  },
  "made_for_you": {
    "limit": 7,
    "discover_weekly": false,
    "release_radar": false
  },
  "recently_played": { "limit": 7 },
  "top_artists": { "visible": false },
  "top_songs": { "visible": false },
  "recommendations": { "visible": false }
}
```

Every section accepts `visible` and `limit` (0 to 255). Setting `visible` to
`false` or `limit` to `0` hides it. Omitted preferences retain the standard
layout. Limits cap loaded items; they do not request more data from Spotify.
Hidden shelves still refresh in the background.

Quick Access defaults to Liked Songs followed by library playlists, up to eight
items. Optional Discover Weekly and Release Radar come next after Liked Songs,
then pinned playlists in local pin order, then library playlists. Duplicates
are omitted. Pins must be playlists present in the loaded library.

Made for You defaults to all four sources: `discover_weekly`, `release_radar`,
`daily_mixes`, and `daylist`. Each can be disabled separately. Recently Played
defaults to 16 items, Top Songs to 10, and Recommendations to 20. Made for You
and Top Artists show all currently fetched items by default (up to 255).
