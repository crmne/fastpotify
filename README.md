# Fastpotify

**Spotify, native and fast.** A lightweight Spotify client written in Rust with
[egui](https://github.com/emilk/egui), playing music through
[librespot](https://github.com/librespot-org/librespot). It runs on Linux,
macOS, and Windows, starts in well under a second, and stays small while it
runs. There is no browser engine anywhere in the process.

**Playing needs Spotify Premium.** Spotify only lets Premium accounts play
music through another app, on the computer itself or on any other device.
With a Free account Fastpotify can browse the library but not play it, and
says so at sign-in.

Fastpotify follows in the footsteps of
[Omarchy Spotify](https://github.com/stappmus/Omarchy-Spotify) and
[spotify-tui](https://github.com/Rigellute/spotify-tui): the familiar Spotify
layout, access to your library, and a Spotify Connect receiver in one desktop
application rather than a shell plugin.

![Fastpotify showing a playlist, with the queue open and a track playing on a remote speaker](docs/screenshot.png)

**Documentation:** [fastpotify.rocks](https://fastpotify.rocks/): what it is, getting started, everyday use, and how it connects to Spotify.

## What it does

- **Plays music on this computer.** Fastpotify is a Spotify Connect device.
  Pick it from your phone, or press play here. Gapless, up to 320 kbps, with
  optional volume normalisation and an on-disk audio cache.
- **Controls other devices.** Move playback to a speaker, a phone, or
  another computer from the device picker, and keep controlling it: play,
  pause, skip, seek, shuffle, repeat, volume.
- **Finds speakers on your network.** A librespot, spotifyd, or hardware
  receiver waiting on the LAN is invisible to Spotify's API until it has an
  account. Fastpotify discovers those over mDNS and connects them for you,
  after which they behave like any other Spotify Connect device.
- **Library access.** Playlists, Liked Songs, saved albums, followed
  artists, podcasts, and saved episodes, filterable in the sidebar and as
  full pages. Sidebar rows pin to the top and drag into your own order.
- **Search** across songs, artists, albums, playlists, podcasts, and episodes,
  with a top result and per-type views.
- **Home** with Made for you, Recently played, your top artists and songs, and
  recommendations.
- **Artist pages** with popular songs, a filterable discography, and related
  artists. **Album**, **playlist**, and **podcast** pages support playback
  from any row.
- **Playlists you own** can be created, renamed, described, reordered, and
  edited: add from any row's menu or by dragging a song onto the playlist in
  the sidebar, remove from the playlist page.
- **Queue** as a side panel or a page; add anything to it from a row menu.
- **Picks up where you left off.** A cold start shows the song the last
  session ended on, paused at the position it stopped at; play carries on
  from there instead of restarting it. Next, previous, and the progress bar
  work on it before it plays, so a skip never has to start it first.
- **Album-art colour.** Pages and the player bar take a tint from the cover
  of what you are looking at or listening to. Turn it off in Settings.
- **Light and dark**, or follow the system.
- **Winamp mini player.** `Ctrl+M` turns the window into a tiny player that
  wears classic `.wsz` skins, drawn pixel for pixel at 1x to 4x, with the
  spectrum analyser, the playlist, and the equalizer hanging under it as
  they did; the logo in the skin brings the big window back. Drop a skin
  from the [Winamp Skin Museum](https://skins.webamp.org) on either window
  to add it.

  ![The mini player wearing the built-in skin](docs/assets/images/winamp.png)
- **Equalizer.** Winamp's ten bands and presets over the music played on
  this computer, in Settings and in the skin.
- **MilkDrop.** Winamp's visualiser, through
  [projectM](https://github.com/projectM-visualizer/projectm), in a
  fullscreenable window of its own (a separate process, so it never disturbs
  the player), playing MilkDrop's own `.milk` presets; Settings fetches the
  packs projectM curates.
- **Keyboard-first.** Every common action has a shortcut (`Ctrl+/` or `?` lists
  them).
- **Keeps playing when you close the window.** The window closes for real,
  the music and the process stay in the system tray (Linux status notifier),
  and clicking the tray, or your desktop's media controls, brings a window
  back. No compositor-specific tricks, so it behaves the same on any
  desktop. Quit from the tray menu or `Ctrl+Q`; turn the behaviour off in
  Settings if you prefer close-to-quit. On macOS the Dock icon stays present,
  and clicking it opens the window again.
- **Visible network activity.** Pages show spinners while they load. An
  indicator appears in the top bar when a Spotify request takes more than a
  moment or is waiting for a rate limit.
- **One instance.** Launching it again brings the existing window forward
  instead of starting a second copy, on every platform.
- **Desktop integration.** MPRIS on Linux, so media keys, the shell, and
  `playerctl` see Fastpotify like any other player. On macOS and Windows,
  `fastpotify next` and its siblings drive the running app from a terminal,
  a launcher, or a hotkey.

## Install

On Arch Linux, Fastpotify is in the AUR:

```bash
yay -S fastpotify-bin      # the released build, ready made
yay -S fastpotify          # the release, built from source
yay -S fastpotify-git      # built from the latest commit
```

On macOS, with [Homebrew](https://brew.sh):

```sh
brew install --cask crmne/tap/fastpotify
```

Everywhere else it is a single binary. Build it with a stable Rust toolchain
(1.95 or newer):

```bash
cargo install --path .
```

MilkDrop is drawn by libprojectM, a C++ library built from source along the
way, so the build also needs CMake, a C++ compiler, and libclang;
`cargo install --path . --no-default-features` leaves MilkDrop out and needs
none of them. On Linux you also need the development packages for ALSA,
PulseAudio (which covers PipeWire), and the usual windowing libraries, for
example on Arch:

```bash
sudo pacman -S --needed alsa-lib libpulse libxkbcommon wayland cmake clang
```

and on Debian or Ubuntu:

```bash
sudo apt install libasound2-dev libpulse-dev libxkbcommon-dev libwayland-dev \
  cmake clang libclang-dev
```

On Windows, libprojectM is built with Visual Studio 2022, CMake, LLVM, and
vcpkg (`vcpkg install glew:x64-windows-static-md`, with
`VCPKG_INSTALLATION_ROOT` pointing at the vcpkg folder).

With [Nix](https://nixos.org), `nix develop` provides all of it, along with
the exact toolchain `rust-toolchain.toml` pins.

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
stores a reusable credential for later sessions. Browsing works on any
account without this step; playing, here or on another device, does not,
because Spotify's API only takes playback commands from Premium accounts.

The Web API always keeps shared catalog coverage. You can also register a
personal Spotify Development Mode app and paste its Client ID in Settings →
Account; supported requests use its separate quota while complete playlist
views, playlist-bearing search, external playlists, and unavailable operations
continue through the shared app.

## Account safety

We are not aware of a Spotify account being suspended for using Fastpotify
or another librespot player with Premium. Sign-in happens on Spotify's own
pages, audio uses the quality included with Premium, DRM stays intact, and
Fastpotify does not rip tracks or block ads.

Reported suspensions usually involve modded apps that remove ads from free
accounts, track ripping, or stream manipulation. Fastpotify does none of
those things, and [CONTRIBUTING.md](CONTRIBUTING.md) prohibits them.

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
| `Ctrl+Shift+K` | MilkDrop, under the mini player |
| `Ctrl+,` | Settings |
| `Ctrl+/` or `?` | All shortcuts |
| `Ctrl+Q` | Quit |

On macOS, `Cmd` replaces `Ctrl`.

## Controlling it from outside

On Linux, Fastpotify is an MPRIS player, so `playerctl --player=fastpotify
play-pause` already works.

macOS and Windows have no such bus, so the same verbs are subcommands. They
talk to the instance already running and print nothing on success:

```
fastpotify play-pause          fastpotify volume 40
fastpotify play                fastpotify volume-up [percent]
fastpotify pause               fastpotify volume-down [percent]
fastpotify next                fastpotify mute
fastpotify previous            fastpotify shuffle [on|off]
fastpotify seek 15             fastpotify repeat [off|context|track]
fastpotify seek -- -15         fastpotify like
fastpotify seek-to 90          fastpotify play-uri spotify:playlist:37i9…
fastpotify show                fastpotify transfer <device-id>
fastpotify now-playing [--raw] fastpotify devices [--raw]
```

`shuffle` and `repeat` toggle when asked for nothing in particular and set
the state outright when given one, which is what a button that draws the
current state wants: a missed update otherwise leaves the two disagreeing
until the next press. `like` saves the playing track to your library, or
takes it back out.

`now-playing` prints one readable line; `--raw` prints the fields
tab-separated (state, title, artists, album, position_ms, duration_ms,
volume, shuffle, repeat, art_url, saved, device) for a script that wants
one of them. `saved` is `yes`, `no`, or `unknown` while the answer is still
on its way. The last three fields were added after the first nine, and
appended rather than woven in, so a script written against the older shape
still reads correctly.

`devices` lists the Spotify Connect devices, id first, the active one
marked with `*`; `--raw` prints them as JSON. The app only refreshes that
list while its own picker is open, so asking for it also asks it to look
again: on a cold list the first call can come back empty and the next one
has it.

A verb exits non-zero when Fastpotify is not running.

Launchers such as Raycast or Alfred can use these commands to control
playback. The Stream Deck plugin speaks the same channel, which is why
the verbs cover more than a media key can ask for.

## Settings

Settings live in one readable JSON file (`~/.config/fastpotify/settings.json`
on Linux). They include the Connect device name, bitrate, normalisation,
autoplay, gapless playback, the audio backend (PulseAudio/PipeWire or ALSA on
Linux), audio cache size, theme, sidebar state, whether pages take colour
from artwork, and the mini player's skin and size.
Playback settings apply when you press **Apply and restart playback**.

Caches (audio, artwork) live under the cache directory and can be deleted at
any time without signing you out.

## How it is built

- `src/player.rs`: the librespot session, player, mixer, and Spirc (Spotify
  Connect) wrapped into one engine that folds player events into a state
  snapshot for the interface.
- `src/api/`: one routing gateway over independent shared and personal Web API
  sessions, each with bounded concurrency and coordinated `Retry-After`
  handling. Capability profiles select current endpoint contracts before a
  request is dispatched.
- `src/backend.rs`: a tokio runtime on its own thread; the interface talks to
  it through channels and is woken with `request_repaint`, so the app is idle
  when nothing happens.
- `src/images.rs`: album art as an egui bytes loader with a disk cache and
  time-based eviction, plus the accent-colour extraction.
- `src/app.rs`, `src/model.rs`, `src/ui/`: state, navigation, and the views.
  Views collect `Action`s while drawing and the app applies them afterwards.
- `src/mpris.rs`: Linux media controls on a dedicated thread.

Fastpotify pins its Rust toolchain in `rust-toolchain.toml`; `cargo test`
covers the API models, dual-session routing, PKCE, the player state machine,
and a headless render of every page, panel, and dialog.

To look at the interface without a Spotify account, build with the `demo`
feature and start it with sample data:

```bash
cargo run --features demo -- --demo --demo-page playlist:pl1 --demo-show queue
```

Demo mode never writes settings. `--demo-shot <PATH>` writes the window to a
PNG and exits, which is how the screenshot above is made. Add
`--demo-size 1240x800` for a repeatable logical window size.

## Contributing

Read [CONTRIBUTING.md](CONTRIBUTING.md) before opening an issue or pull
request. It describes the project's design principles, product boundaries,
and the complete local checks that every change must pass.

## Acknowledgements

Fastpotify stands on [librespot](https://github.com/librespot-org/librespot),
[egui](https://github.com/emilk/egui), the [Inter](https://rsms.me/inter/)
typeface (OFL), and [Lucide](https://lucide.dev) icons (ISC).

Fastpotify is an independent project and is not affiliated with Spotify.
Spotify is a trademark of Spotify AB.

Licensed under the [MIT License](LICENSE).
