# Vendored yt-dlp

Release binaries embed **one** official yt-dlp build for the target they
are compiled for. The large files under `bin/` are not committed.

```
pwsh -File third_party/yt-dlp/vendor.ps1 -Target x86_64-pc-windows-msvc
sh third_party/yt-dlp/vendor.sh --target x86_64-unknown-linux-gnu
```

Omit `-Target` / `--target` to vendor the host triple. `--require`
(or `-Require`) exits non-zero if the asset is missing after the run;
the release workflow uses that.

`FASTPOTIFY_SKIP_YTDLP_BUNDLE=1` builds without embedding even when the
file is present. A missing file on an ordinary dev/CI build is not an
error: Piped and an external yt-dlp still work. A hash mismatch is a
build failure.
