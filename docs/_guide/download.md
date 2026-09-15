---
title: Download
description: Download the app for macOS, Windows, or Linux, with install instructions for each.
nav_order: 1
---

Spotifast was previously called **Fastpotify**. Version 0.8.0 introduces the
new name. Install packages named `spotifast`; your settings and sign-ins carry over.

{% assign v = site.fastpotify_version %}
{% assign base = "https://github.com/crmne/spotifast/releases/download/v" | append: v %}

The current version is **v{{ v }}**. SHA-256 checksums are in
[checksums.txt]({{ base }}/checksums.txt). Older versions are on the
[releases page](https://github.com/crmne/spotifast/releases).

## macOS

One download for both Apple Silicon and Intel:

- [spotifast-v{{ v }}-macos-universal.dmg]({{ base }}/spotifast-v{{ v }}-macos-universal.dmg)

Open it and drag **Spotifast** to Applications. Once opened, it is
registered for `spotify:` links, so links shared from other apps open in
it; with the official client installed too, macOS keeps whichever it used
last. Or, with [Homebrew](https://brew.sh):

```sh
brew install --cask crmne/tap/spotifast
```

### First open on macOS

Version 0.8.0 is signed with Developer ID and notarized by Apple. Open
**Spotifast** from Applications and confirm the normal downloaded-app prompt.
No quarantine-removal command or security exception is needed.

When upgrading from 0.7.1, quit Fastpotify before opening Spotifast. Both use
the same saved settings and sign-ins.

## Windows

The installer adds Spotifast to the Start menu and needs no administrator
rights. It also registers Spotifast for `spotify:` links; if the official
client is installed too, Settings → Apps → Default apps decides which of
the two opens them. Choose x86_64 for most PCs or aarch64 for Windows on ARM:

- [spotifast-v{{ v }}-x86_64-pc-windows-msvc-setup.exe]({{ base }}/spotifast-v{{ v }}-x86_64-pc-windows-msvc-setup.exe)
- [spotifast-v{{ v }}-aarch64-pc-windows-msvc-setup.exe]({{ base }}/spotifast-v{{ v }}-aarch64-pc-windows-msvc-setup.exe)

For a portable copy, download a zip, unpack it, and run `spotifast.exe`.

- [spotifast-v{{ v }}-x86_64-pc-windows-msvc.zip]({{ base }}/spotifast-v{{ v }}-x86_64-pc-windows-msvc.zip)
- [spotifast-v{{ v }}-aarch64-pc-windows-msvc.zip]({{ base }}/spotifast-v{{ v }}-aarch64-pc-windows-msvc.zip)

Either way, SmartScreen may warn about an unknown publisher on first run;
choose More info, then Run anyway.

## Linux

### Arch Linux

Spotifast is in the AUR, with the desktop entry and icon installed for you:

```sh
yay -S spotifast-bin      # the released build, ready made
yay -S spotifast          # the release, built from source
yay -S spotifast-git      # built from the latest commit
```

If you already have an old `fastpotify` package, install the corresponding
`spotifast` package above and accept the replacement. The old packages
announce this migration in a packaging-only revision. No settings or saved
sign-ins are removed.

### Flatpak

The release carries a [Spotifast Flatpak bundle]({{ base }}/spotifast-v{{ v }}-x86_64.flatpak?flatpak-id=rocks.spotifast.Spotifast)
of the Linux build. It runs on
any distribution with Flatpak and the Freedesktop 24.08 runtime:

```sh
flatpak install --user ~/Downloads/spotifast-v{{ v }}-x86_64.flatpak
flatpak run rocks.spotifast.Spotifast
```

The application ID is **`rocks.spotifast.Spotifast`**. Existing Fastpotify
Flatpak users install this as a new application, then remove the old one.
See [switching Flatpak installations](/renaming/#flatpak) to retain settings
and history. Sign in again after switching.

A bundle does not update itself. Flathub support is planned.

The bundle uses the same binary as the release tarball. Other stores use
third-party packages. Report package-specific problems to their packagers.

### Other distributions

- [spotifast-v{{ v }}-x86_64-unknown-linux-gnu.tar.gz]({{ base }}/spotifast-v{{ v }}-x86_64-unknown-linux-gnu.tar.gz)
- [spotifast-v{{ v }}-aarch64-unknown-linux-gnu.tar.gz]({{ base }}/spotifast-v{{ v }}-aarch64-unknown-linux-gnu.tar.gz)

Unpack, put `spotifast` on your PATH, and copy the desktop entry and icon
from the bundled `packaging/` directory if you want it in your launcher and
handling `spotify:` links.
The binary needs ALSA, PulseAudio or PipeWire, and Wayland or X11.

Or build from source: see [Getting Started](/getting-started/).

## Nix

Add the repository [flake](https://github.com/crmne/spotifast) to your
inputs:

```nix
inputs.spotifast.url = "github:crmne/spotifast";
```

On NixOS, install the default package:

```nix
environment.systemPackages = [
  inputs.spotifast.packages."${pkgs.stdenv.hostPlatform.system}".default
];
```

### nix-darwin

On macOS, install the `spotifast` package. It includes the native binary and
a locally built, ad-hoc signed `Spotifast.app` bundle, so it is never
quarantined and the first-open steps above do not apply:

```nix
environment.systemPackages = [
  inputs.spotifast.packages."${pkgs.stdenv.hostPlatform.system}".spotifast
];
environment.pathsToLink = [ "/Applications" ];
```

The bundle appears in `/Applications/Nix Apps`. With Home Manager,
`home.packages` is enough; its darwin support links app bundles into
`~/Applications`:

```nix
home.packages = [
  inputs.spotifast.packages."${pkgs.stdenv.hostPlatform.system}".spotifast
];
```
