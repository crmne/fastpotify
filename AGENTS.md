# Fastpotify agent guide

Follow `CONTRIBUTING.md`; it is the canonical product and contribution policy.
These instructions add implementation constraints for coding agents.

## Product boundaries

- Keep Fastpotify a small native Spotify client. Do not add a browser engine,
  telemetry, a hosted backend, or alternate sources for Spotify audio.
- Playback capabilities come from librespot. Do not advertise or implement a
  capability merely because its name appears in a protobuf or enum. In
  particular, do not pursue Spotify Lossless or DRM circumvention unless
  lawful support first lands upstream.
- Do not broaden a task into adjacent features or a general refactor. Preserve
  existing user behaviour unless the task explicitly changes it.

## Architecture

- `src/ui/` draws views and emits `Action`s. Apply actions after drawing in
  `src/app.rs`; do not mutate application state from inside a borrowed view.
- Network and playback work belongs on the runtime in `src/backend.rs` or in
  the player engine in `src/player.rs`, never as blocking work on the UI
  thread.
- Keep platform integrations behind target-specific modules or `cfg` blocks.
  A fix for one platform must keep the other two targets compiling.
- Settings and state files must remain readable, backward compatible, and
  atomically written. Never log credentials or authorization responses.
- Prefer existing dependencies. Explain any new crate in `Cargo.toml` next to
  the dependency when the reason is not obvious.

Read `docs/_reference/how-it-connects.md` before changing authentication,
Spotify requests, Connect, credential storage, or network behaviour. Read
`docs/_reference/queue.md` before touching the queue: its rules are the
contract, and the queue tests in `src/app.rs` enforce them. Read the
nearby module tests before changing a state machine or API fallback.

The interface is optimistic, always. A control shows its result the
moment it is used: a double-clicked song is the playing song, Next pops
the queue's head, an added song has its row. The backend then makes it
true and Spotify's state catches up behind; an answer that still tells
the story from before the user's action is stale, so hold the shown
state and ask again rather than let the lagging answer undo what the
user just did. Nothing the user did may ever flicker away and come back.

Every visualiser, the spectrum analyser, the oscilloscope, and MilkDrop,
shows the signal post-equalizer and pre-volume: the EQ shapes what is
heard so the picture follows it, and the volume knob never moves the
picture. Zero volume still dances.

## Branches

Work on `main`. Commit there directly, one topic per commit, each
compiling and passing the checks on its own. Feature branches and pull
requests are for outside contributors; the maintainer's own work, and
work done with the maintainer, does not go through them.

## Definition of done

- Add focused regression tests for changed behaviour. Use the `demo` feature
  for deterministic UI coverage and screenshots.
- Update the README and docs when user-visible behaviour, settings, files, or
  network access changes.
- Run the full checks from `CONTRIBUTING.md`. Do not weaken a lint, delete a
  test, or add an `allow` merely to make CI green without explaining why the
  underlying rule does not apply.
- Report platform coverage honestly. Do not claim a platform was tested when
  it was only compiled or reasoned about.

## Releases

A release is not the tag alone. Do these in order:

1. Change the `Cargo.toml` version and update the lockfile with a build.
   Commit and push this before the tag so the binaries report the right
   version.
2. Push the `v*` tag, which triggers the release workflow. Wait for every
   required artifact and `checksums.txt`, then replace the generated notes
   with written ones.
3. A prerelease stops here. Keep the stable version current on the website,
   Homebrew, and AUR. The prerelease remains available from GitHub's releases
   page.
4. For a stable release, only after the GitHub release exists, update
   `docs/_config.yml` `fastpotify_version` and
   `docs/_data/versions.yml`. The selector carries only the latest stable
   version: replace its version entry, make it `current`, and point it at
   `/download/`. Do not retain older version entries; they remain available
   through the Changelog link. Never make the download page point at files
   that do not exist yet.
5. Update the Homebrew cask in the maintainer's tap and the AUR package from
   the release's `checksums.txt`.

Skipping an applicable step ships a release that lies somewhere; the dropdown
was forgotten once already.
