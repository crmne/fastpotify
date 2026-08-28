# Fastpotify

**Spotify, native and fast.** A lightweight Spotify client written in Rust with
[egui](https://github.com/emilk/egui), playing music through
[librespot](https://github.com/librespot-org/librespot). It runs on Linux,
macOS, and Windows, starts in well under a second, and stays small while it
runs. There is no browser engine anywhere in the process.

Fastpotify follows in the footsteps of
[Omarchy Spotify](https://github.com/stappmus/Omarchy-Spotify) and
[spotify-tui](https://github.com/Rigellute/spotify-tui): the familiar Spotify
layout, the whole library, and a Spotify Connect receiver on your computer,
as one ordinary desktop application rather than a shell plugin.

![Fastpotify showing a playlist, with the queue open and a track playing on a remote speaker](docs/screenshot.png)

**Documentation:** [fastpotify.rocks](https://fastpotify.rocks/): what it is, getting started, everyday use, and how it connects to Spotify.

## What it does

- **Plays music on this computer.** Fastpotify is a Spotify Connect device.
  Pick it from your phone, or press play here. Gapless, up to 320 kbps, with
  optional volume normalisation and an on-disk audio cache.
- **Controls every other device.** Move playback to a speaker, a phone, or
  another computer from the device picker, and keep controlling it: play,
  pause, skip, seek, shuffle, repeat, volume.
- **Finds speakers on your network.** A librespot, spotifyd, or hardware
  receiver waiting on the LAN is invisible to Spotify's API until it has an
  account. Fastpotify discovers those over mDNS and connects them for you,
  after which they behave like any other Spotify Connect device.
- **Your whole library.** Playlists, Liked Songs, saved albums, followed
  artists, podcasts, and saved episodes, filterable in the sidebar and as
  full pages.
- **Search** across songs, artists, albums, playlists, podcasts, and episodes,
  with a top result and per-type views.
- **Home** with Made for you, Recently played, your top artists and songs, and
  recommendations.
- **Artist pages** with popular songs, a filterable discography, and related
  artists. **Album**, **playlist**, and **podcast** pages with everything
  playable from any row.
- **Playlists you own** can be created, renamed, described, reordered, and
  edited: add from any row's menu, remove from the playlist page.
- **Queue** as a side panel or a page; add anything to it from a row menu.
- **Album-art colour.** Pages and the player bar take a tint from the cover
  of what you are looking at or listening to. Turn it off in Settings.
- **Light and dark**, or follow the system.
- **Keyboard-first.** Every common action has a shortcut (`Ctrl+/` lists
  them).
- **Keeps playing when you close the window.** The window closes for real,
  the music and the process stay in the system tray (Linux status notifier),
  and clicking the tray, or your desktop's media controls, brings a window
  back. No compositor-specific tricks, so it behaves the same on any
  desktop. Quit from the tray menu or `Ctrl+Q`; turn the behaviour off in
  Settings if you prefer close-to-quit.
- **Honest about the network.** Pages show spinners while they load, a
  quiet indicator appears in the top bar whenever the app is talking to
  Spotify for more than a moment, and if Spotify asks the app to back off
  you see that it is waiting, instead of an unexplained pause.
- **One instance.** Launching it again surfaces the window that is already
  open instead of starting a rival copy, on every platform.
- **Desktop integration.** MPRIS on Linux, so media keys, the shell, and
  `playerctl` see Fastpotify like any other player.

## Install

On Arch Linux, Fastpotify is in the AUR:

```bash
yay -S fastpotify          # the released build
yay -S fastpotify-git      # built from the latest commit
```

Everywhere else it is a single binary. Build it with a stable Rust toolchain
(1.95 or newer):

```bash
cargo install --path .
```

On Linux you also need the development packages for ALSA, PulseAudio (which
covers PipeWire), and the usual windowing libraries, for example on Arch:

```bash
sudo pacman -S --needed alsa-lib libpulse libxkbcommon wayland
```

and on Debian or Ubuntu:

```bash
sudo apt install libasound2-dev libpulse-dev libxkbcommon-dev libwayland-dev
```

Titles in a script the interface font does not cover -- Chinese, Japanese,
Korean, Arabic, Hebrew, Thai, the Indic scripts and a dozen more -- are drawn
with a face borrowed from the system rather than bundled, which would cost
more than ten megabytes for Chinese alone. macOS and Windows carry faces for
the common ones; on Linux install the Noto families for the scripts you
listen to, for example `noto-fonts` and `noto-fonts-cjk` (Arch) or
`fonts-noto` and `fonts-noto-cjk` (Debian or Ubuntu). A script with no face
installed still shows as empty boxes.

A desktop entry is provided in `packaging/applications/fastpotify.desktop`.

## Sign in

Press **Sign in with Spotify**. Your browser opens Spotify's own consent
page (Authorization Code with PKCE); Fastpotify never sees your password.
When Spotify redirects back to the app, your library, search, and control
of other devices work immediately. The refresh token is stored in the
platform's state directory (`~/.local/state/fastpotify` on Linux), so the
browser is needed once per machine.

Playing music **on this computer** is one more one-time browser approval.
Spotify treats streaming as a separate grant for its own client identity,
which is what librespot plays with. Take it from the device menu ("Play
here, set up once") or Settings; it needs Spotify Premium, and librespot
stores a reusable credential so it also never asks again. Browsing and
remote control work on any account without this step.

By default the Web API uses the shared public application also used by
spotify-player, ncspot, and Omarchy Spotify. If you hit rate limits you can
register your own (free) Spotify application and paste its Client ID in
Settings → Account.

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
| `Alt+←` / `Alt+→` | Back or forward |
| `Ctrl+H` / `Ctrl+L` | Home / Liked Songs |
| `Ctrl+Shift+A` / `Ctrl+Shift+B` | Playing artist / album |
| `Ctrl+,` | Settings |
| `Ctrl+/` | All shortcuts |
| `Ctrl+Q` | Quit |

On macOS, `Cmd` replaces `Ctrl`.

## Settings

Everything lives in one readable JSON file (`~/.config/fastpotify/settings.json`
on Linux): the Connect device name, bitrate, normalisation, autoplay, gapless
playback, the audio backend (PulseAudio/PipeWire or ALSA on Linux), audio
cache size, theme, and whether pages take colour from artwork. Playback
settings apply when you press **Apply and restart playback**.

Caches (audio, artwork) live under the cache directory and can be deleted at
any time without signing you out.

## How it is built

- `src/player.rs`: the librespot session, player, mixer, and Spirc (Spotify
  Connect) wrapped into one engine that folds player events into a state
  snapshot for the interface.
- `src/api/`: a small Web API client with bounded concurrency,
  `Retry-After` handling, and automatic fallback between the 2026 endpoint
  shapes (`/me/library`, `/playlists/{id}/items`) and the classic ones.
- `src/backend.rs`: a tokio runtime on its own thread; the interface talks to
  it through channels and is woken with `request_repaint`, so the app is idle
  when nothing happens.
- `src/images.rs`: album art as an egui bytes loader with a disk cache and
  time-based eviction, plus the accent-colour extraction.
- `src/app.rs`, `src/model.rs`, `src/ui/`: state, navigation, and the views.
  Views collect `Action`s while drawing and the app applies them afterwards.
- `src/mpris.rs`: Linux media controls on a dedicated thread.

Fastpotify pins its Rust toolchain in `rust-toolchain.toml`; `cargo test`
covers the API models, the endpoint fallbacks, PKCE, the player state
machine, and a headless render of every page, panel, and dialog.

To look at the interface without a Spotify account, build with the `demo`
feature and start it with sample data:

```bash
cargo run --features demo -- --demo --demo-page playlist:pl1 --demo-show queue
```

Demo mode never writes settings. `--demo-shot <PATH>` writes the window to a
PNG and exits, which is how the screenshot above is made.

## Acknowledgements

Fastpotify stands on [librespot](https://github.com/librespot-org/librespot),
[egui](https://github.com/emilk/egui), the [Inter](https://rsms.me/inter/)
typeface (OFL), and [Lucide](https://lucide.dev) icons (ISC).

Fastpotify is an independent project and is not affiliated with Spotify.
Spotify is a trademark of Spotify AB.

Licensed under the [MIT License](LICENSE).
