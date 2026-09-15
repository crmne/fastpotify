---
title: The Rename
description: How existing installations, commands, settings and links survive the new name.
---

Fastpotify is now **Spotifast**, at [spotifast.rocks](https://spotifast.rocks/).
Version 0.8.0 is the first release with the new name. Releases through 0.7.1
use the Fastpotify name.

## Existing installations

Settings, saved sign-ins, local pins, history, caches and window positions stay
where they are for native packages. Flatpak's new application ID uses a new
data directory, with migration instructions below. Saved
Spotify Connect names also stay as you chose them; new settings use Spotifast.

The `spotifast` and `fastpotify` commands open and control the same application.
Linux packages provide `spotifast` as an alias, without installing a second
copy of the app. Existing `playerctl --player=fastpotify` commands keep working.

On `main`, after 0.8.0, native Linux launchers use `spotifast.desktop`, with
`Icon=spotifast` and `StartupWMClass=spotifast`. Wayland windows and the desktop
entry reported through MPRIS use the same identity. Flatpak uses its full
`rocks.spotifast.Spotifast` ID instead. The main window still reads its existing
`fastpotify/app.ron` state, so geometry and interface preferences survive.
The mini player's state remains separate.

After installing the updated launcher, select it again for any pinned desktop
shortcut or custom launcher command that explicitly names `fastpotify.desktop`.
Run `xdg-mime default spotifast.desktop x-scheme-handler/spotify` to choose it
for Spotify links. Published 0.8.0 packages retain their old desktop filename
and window identity until an application update; their executable bytes are
unchanged by the package rename.

AUR packages are now `spotifast`, `spotifast-bin` and `spotifast-git`.
The old packages have a packaging-only update that announces the move.
Install the matching new package and accept the replacement, for example:

```sh
yay -S spotifast-bin
```

There is no need to uninstall first or remove settings. The source and binary
release packages still use the same 0.8.0 application code. The `-git` variant
continues to build the current development revision.

The Homebrew cask is now `crmne/tap/spotifast`. Its rename metadata lets
Homebrew migrate existing installations during updates, or explicitly with
`brew migrate --cask fastpotify` after updating the tap. DEB/RPM packages are
also named `spotifast` and declare replacement of `fastpotify`.
Nix exposes `spotifast` as the package for both the command and, on macOS,
the `Spotifast.app` bundle, alongside the old `fastpotify` attribute.
Community-maintained distribution packages may still use the old name until
their maintainers update them.

## Updates and packaging

Public release downloads use the `spotifast-` prefix. The 0.8.0 native downloads
were renamed without changing their bytes. Flatpak was repackaged with its
new application ID while retaining the original executable. Byte-identical
native `fastpotify-` compatibility downloads remain for installed update clients that request
those exact filenames, with both names recorded in `checksums.txt`. The compatibility command keeps its `fastpotify VERSION`
response. The Spotifast command reports `spotifast VERSION`; new update clients
accept either name and still require the exact expected version and checksum.

New macOS installations use `Spotifast.app`. The bundle ID remains
`me.paolino.fastpotify`, and its internal executable remains `fastpotify`.
The disk image also includes a hidden, signed copy named `Fastpotify.app` for
older updaters that require that path. Updating an existing installation
preserves its current bundle location. Homebrew updates remain owned by
Homebrew, whichever bundle name is installed.

Windows keeps its original installer ID, registry identities and installation
directory. Its app name and new shortcuts say Spotifast. The previous command
remains installed for existing shortcuts and scripts.

The protected credential-store service, MPRIS bus name and single-instance
protocol retain their original identities for existing integrations.

## Flatpak

The Spotifast Flatpak bundle uses **`rocks.spotifast.Spotifast`**. This is a separate
Flatpak application, so install the new bundle and remove the old application.
The repackaged 0.8.0 bundle is available from the [download page](/download/)
and contains the original 0.8.0 executable.

To retain settings, local pins and history, quit the old application. Before
the first launch of the new one, copy its data directory:

```sh
old_data="$HOME/.var/app/rocks.fastpotify.Fastpotify"
new_data="$HOME/.var/app/rocks.spotifast.Spotifast"
test -d "$old_data" && test ! -e "$new_data" && cp -a "$old_data" "$new_data"
```

This leaves the original directory intact and refuses to overwrite an existing
new profile. If you have already opened the new application, retain that profile
or move it aside before copying. Protected sign-ins are scoped to the original
state directory, so sign in again after switching.

Install the new bundle from the [download page](/download/), then run it:

```sh
flatpak install --user ~/Downloads/spotifast-v0.8.0-x86_64.flatpak
flatpak run rocks.spotifast.Spotifast
```

After checking the new installation, remove the old one:

```sh
flatpak uninstall --user rocks.fastpotify.Fastpotify
```

Use `--system` instead of `--user` if the old application was installed
system-wide. Uninstalling without `--delete-data` retains its data for recovery.

## Website links

The guides now live at `/using-spotifast/` and `/what-is-spotifast/`.
`jekyll-redirect-from` generates redirects from their old Fastpotify URLs.
The download page continues to link to the existing stable artifacts until a
new release is available.
