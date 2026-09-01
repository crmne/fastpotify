---
title: Download
description: Get Fastpotify for macOS, Windows, or Linux, with install instructions for each.
nav_order: 1
---

{% assign v = site.fastpotify_version %}
{% assign base = "https://github.com/crmne/fastpotify/releases/download/v" | append: v %}

The current version is **v{{ v }}**. Every file below, with its SHA-256, is
listed in [checksums.txt]({{ base }}/checksums.txt); all versions live on
the [releases page](https://github.com/crmne/fastpotify/releases).

## macOS

One download for both Apple Silicon and Intel:

- [fastpotify-v{{ v }}-macos-universal.dmg]({{ base }}/fastpotify-v{{ v }}-macos-universal.dmg)

Open it and drag **Fastpotify** to Applications. Or, with
[Homebrew](https://brew.sh):

```sh
brew install --cask crmne/tap/fastpotify
```

Homebrew installs the same unnotarized build, so the first-open steps below
still apply. To skip them, clear the quarantine flag instead:

```sh
xattr -dr com.apple.quarantine /Applications/Fastpotify.app
```

The `-r` matters: it clears the flag from the files inside the bundle too.
macOS 26 leaves the app bouncing in the Dock forever when only the top level
is cleared.

### First open on macOS

This build is not yet notarized with Apple, so macOS blocks it the first
time. Recent macOS versions (Sequoia and later) no longer let you bypass
this with a right-click, so you open it once through Privacy & Security:

1. Double-click **Fastpotify** in Applications. macOS says it cannot be
   opened because Apple cannot check it for malicious software. Click
   **Done** (do **not** click Move to Trash).
2. Open **System Settings**, then **Privacy & Security**.
3. Scroll down to the **Security** section, find *"Fastpotify was blocked
   to protect your Mac"*, and click **Open Anyway**.
4. Authenticate, then click **Open Anyway** once more.

macOS remembers the choice, so later launches work with an ordinary
double-click.

## Windows

The installer adds Fastpotify to the Start menu and needs no administrator
rights. Choose x86_64 for most PCs or aarch64 for Windows on ARM:

- [fastpotify-v{{ v }}-x86_64-pc-windows-msvc-setup.exe]({{ base }}/fastpotify-v{{ v }}-x86_64-pc-windows-msvc-setup.exe)
- [fastpotify-v{{ v }}-aarch64-pc-windows-msvc-setup.exe]({{ base }}/fastpotify-v{{ v }}-aarch64-pc-windows-msvc-setup.exe)

If you would rather not install anything, the same program comes as a zip:
unpack it and run `fastpotify.exe`.

- [fastpotify-v{{ v }}-x86_64-pc-windows-msvc.zip]({{ base }}/fastpotify-v{{ v }}-x86_64-pc-windows-msvc.zip)
- [fastpotify-v{{ v }}-aarch64-pc-windows-msvc.zip]({{ base }}/fastpotify-v{{ v }}-aarch64-pc-windows-msvc.zip)

Either way, SmartScreen may warn about an unknown publisher on first run;
choose More info, then Run anyway.

## Linux

### Arch Linux

Fastpotify is in the AUR, with the desktop entry and icon installed for you:

```sh
yay -S fastpotify-bin      # the released build, ready made
yay -S fastpotify          # the release, built from source
yay -S fastpotify-git      # built from the latest commit
```

### Flatpak

From 0.4.0 on, every release carries a Flatpak bundle of the Linux build,
`fastpotify-vX.Y.Z-x86_64.flatpak`, on the
[releases page](https://github.com/crmne/fastpotify/releases). It runs on
any distribution with Flatpak and the Freedesktop 24.08 runtime:

```sh
flatpak install --user ~/Downloads/fastpotify-vX.Y.Z-x86_64.flatpak
```

A bundle does not update itself; a Flathub listing, which would, is in the
works.

That bundle is the one built here, from the same binary as the tarball
above. Fastpotify on other stores is packaged by other people and is not
supported: if one of those will not start or will not play, the packager
is the person who can fix it, so please raise it with them.

### Other distributions

- [fastpotify-v{{ v }}-x86_64-unknown-linux-gnu.tar.gz]({{ base }}/fastpotify-v{{ v }}-x86_64-unknown-linux-gnu.tar.gz)
- [fastpotify-v{{ v }}-aarch64-unknown-linux-gnu.tar.gz]({{ base }}/fastpotify-v{{ v }}-aarch64-unknown-linux-gnu.tar.gz)

Unpack, put `fastpotify` on your PATH, and copy the desktop entry and icon
from the bundled `packaging/` directory if you want it in your launcher.
Runtime needs are the ordinary desktop libraries: ALSA, PulseAudio or
PipeWire, and Wayland or X11.

Or build from source: see [Getting Started](/getting-started/).
