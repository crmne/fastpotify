# Contributing Spanish (Spain) locale upstream

See `dist/UPSTREAM-PR.md` after a local build for the installer and zip.

This document is the PR text for [crmne/fastpotify](https://github.com/crmne/fastpotify).

## Title

Add Spanish (Spain) interface locale

## Body

### Summary

- Adds a lightweight i18n layer (`src/i18n.rs`) with embedded `locales/en.json` and `locales/es-ES.json` (~551 UI strings).
- Adds a **Language** setting (Follow system / English / Español España) under Settings → Appearance.
- System locale detection: `es` and `es-ES` map to Spanish (Spain); everything else defaults to English.
- Translates the full interface: navigation, settings, dialogs, toasts, tray menu, macOS menu, auth browser pages, and Winamp mini player.
- Backend messages use translation keys resolved in the UI via `i18n::translate()`.
- Includes `scripts/gen_locales.py` to regenerate locale files and verify key parity between languages.

### Why

Fastpotify had no localization. Spanish is one of the largest Spotify markets; this gives users a complete Castilian UI without new runtime dependencies (strings are compiled in with `include_str!`).

### Test plan

- [x] `cargo fmt --all --check`
- [x] `cargo test --locked --all-targets` (157 tests)
- [ ] Manual: Settings → Appearance → Language → Español (España)
- [ ] Manual: Follow system on a Spanish Windows locale
- [ ] Screenshots attached
