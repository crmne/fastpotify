---
title: Download
description: Get Fastpotify for macOS, Windows, or Linux, with install instructions for each.
nav_order: 1
---

{% assign v = site.fastpotify_version %}
{% assign base = "https://github.com/crmne/fastpotify/releases/download/v" | append: v %}

The current version is **v{{ v }}**. SHA-256 checksums are in
[checksums.txt]({{ base }}/checksums.txt). Older versions are on the
[releases page](https://github.com/crmne/fastpotify/releases).

## macOS

One download for both Apple Silicon and Intel:

- [fastpotify-v{{ v }}-macos-universal.dmg]({{ base }}/fastpotify-v{{ v }}-macos-universal.dmg)

Open it and drag **Fastpotify** to Applications. Once opened, it is
registered for `spotify:` links, so links shared from other apps open in
it; with the official client installed too, macOS keeps whichever it used
last. Or, with [Homebrew](https://brew.sh):

```sh
brew install --cask crmne/tap/fastpotify
```

Homebrew installs the same unnotarized build, so the first-open steps below
still apply. To skip them, clear the quarantine flag instead:

```sh
find /Applications/Fastpotify.app -exec xattr -d com.apple.quarantine {} \; 2>/dev/null
```

The command must clear every file in the bundle. Clearing only the app can
leave it bouncing in the Dock on macOS 26. This command also works on macOS
27, where `xattr` no longer accepts `-r`.

If the command fails, use the steps below instead. They do not need a
terminal.

### First open on macOS

This build is not notarized, so macOS blocks the first launch. On Sequoia and
later, allow it in Privacy & Security:

1. Double-click **Fastpotify** in Applications. macOS says it cannot be
   opened because Apple cannot check it for malicious software. Click
   **Done** (do **not** click Move to Trash).
2. Open **System Settings**, then **Privacy & Security**.
3. Scroll down to the **Security** section, find *"Fastpotify was blocked
   to protect your Mac"*, and click **Open Anyway**.
4. Authenticate, then click **Open Anyway** once more.

Later launches work with a normal double-click.

## Windows

The installer adds Fastpotify to the Start menu and needs no administrator
rights. It also registers Fastpotify for `spotify:` links; if the official
client is installed too, Settings → Apps → Default apps decides which of
the two opens them. Choose x86_64 for most PCs or aarch64 for Windows on ARM:

- [fastpotify-v{{ v }}-x86_64-pc-windows-msvc-setup.exe]({{ base }}/fastpotify-v{{ v }}-x86_64-pc-windows-msvc-setup.exe)
- [fastpotify-v{{ v }}-aarch64-pc-windows-msvc-setup.exe]({{ base }}/fastpotify-v{{ v }}-aarch64-pc-windows-msvc-setup.exe)

For a portable copy, download a zip, unpack it, and run `fastpotify.exe`.

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

A bundle does not update itself. Flathub support is planned.

The bundle uses the same binary as the release tarball. Other stores use
third-party packages. Report package-specific problems to their packagers.

### Other distributions

- [fastpotify-v{{ v }}-x86_64-unknown-linux-gnu.tar.gz]({{ base }}/fastpotify-v{{ v }}-x86_64-unknown-linux-gnu.tar.gz)
- [fastpotify-v{{ v }}-aarch64-unknown-linux-gnu.tar.gz]({{ base }}/fastpotify-v{{ v }}-aarch64-unknown-linux-gnu.tar.gz)

Unpack, put `fastpotify` on your PATH, and copy the desktop entry and icon
from the bundled `packaging/` directory if you want it in your launcher and
handling `spotify:` links.
The binary needs ALSA, PulseAudio or PipeWire, and Wayland or X11.

Or build from source: see [Getting Started](/getting-started/).
