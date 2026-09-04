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

`docs/_reference/what-spotify-allows.md` lists what the Web API, the
librespot session, and librespot playback each offer, and the requests
none of them can serve (pins synchronised with Spotify, folder editing,
Smart Shuffle, lossless, local files, and more), each with its reason.
Before building or promising a Spotify-facing feature, and before answering
an issue that asks for one, find it there. A request in the last section is
answered with that reason and closed, not worked on; if the reason has
lapsed because librespot or the Web API gained the capability, update the
page in the same change.

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

## Issue communication

- Write public replies for the reporter, not as an engineering investigation
  log. Keep them short, direct, and in plain language.
- A reply should move the issue forward: make the maintainer's decision, say
  that a fix is planned or in progress, or ask for one specific thing needed
  next. Include technical detail only when the reporter needs it to act.
- When a valid issue has a clear, bounded fix that can be implemented now,
  implement it instead of posting the proposed design in the issue. Do not use
  public comments as notes to yourself or as a substitute for doing the work.
- Never post two maintainer comments in a row on the same issue or pull
  request. If nobody has replied since the last maintainer comment, edit that
  comment instead.
- Keep private investigation notes out of the public thread. Do not post a
  second comment merely to document more analysis.
- Never use em dashes. Use a full stop, comma, colon, or parentheses instead.

## Interface review

- Distinguish an internal UI refactor from an interface redesign. Moving
  navigation or controls, regrouping menus, changing the application shell,
  window chrome, panel ownership or sizing, responsive breakpoints, spacing,
  or visual hierarchy is a redesign even when behavior still works.
- Call out every user-visible interface change at the top of a pull request
  review. Correct code and green CI do not make a redesign merge-ready.
- Require explicit maintainer approval of the visual scope before merging an
  interface redesign. Conditional approval to assess code quality is not
  approval of changed appearance or interaction.
- Inspect before-and-after evidence at representative window sizes and in both
  light and dark themes. If that evidence is missing, request it.

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

Before writing release notes, read the previous two stable releases and match
their style. Start with a short plain-language summary, use `New` and `Fixed`
sections as applicable, lead each item with a bold user-facing result, credit
contributors and reporters with the relevant issue or pull request numbers,
include a `Thanks` section, and end with the full changelog link. Do not leave
the generated notes in place or introduce a different section scheme for
ordinary improvements.

Skipping an applicable step ships a release that lies somewhere; the dropdown
was forgotten once already.
