#!/usr/bin/env sh
# Download one official yt-dlp asset for a cargo target. Runtime never downloads.
set -eu

ROOT=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
MANIFEST="$ROOT/manifest"
TARGET=""
REQUIRE=0
# POSIX CR strip for Windows checkouts. $'\r' is bash-only.
CR=$(printf '\r')

usage() {
  echo "usage: vendor.sh [--target TRIPLE] [--require] [--manifest PATH]" >&2
  exit 2
}

while [ $# -gt 0 ]; do
  case "$1" in
    --target)
      [ $# -ge 2 ] || usage
      TARGET=$2
      shift 2
      ;;
    --require)
      REQUIRE=1
      shift
      ;;
    --manifest)
      [ $# -ge 2 ] || usage
      MANIFEST=$2
      shift 2
      ;;
    -h|--help)
      usage
      ;;
    *)
      usage
      ;;
  esac
done

detect_target() {
  os=$(uname -s 2>/dev/null || echo unknown)
  arch=$(uname -m 2>/dev/null || echo unknown)
  case "$os" in
    Linux)
      case "$arch" in
        x86_64) echo x86_64-unknown-linux-gnu ;;
        aarch64|arm64) echo aarch64-unknown-linux-gnu ;;
        *) echo "" ;;
      esac
      ;;
    Darwin)
      case "$arch" in
        x86_64) echo x86_64-apple-darwin ;;
        aarch64|arm64) echo aarch64-apple-darwin ;;
        *) echo "" ;;
      esac
      ;;
    MINGW*|MSYS*|CYGWIN*|Windows_NT)
      case "$arch" in
        x86_64|AMD64) echo x86_64-pc-windows-msvc ;;
        aarch64|arm64|ARM64) echo aarch64-pc-windows-msvc ;;
        *) echo "" ;;
      esac
      ;;
    *)
      echo ""
      ;;
  esac
}

if [ -z "$TARGET" ]; then
  TARGET=$(detect_target)
fi
if [ -z "$TARGET" ]; then
  echo "vendor.sh: cannot detect host target; pass --target" >&2
  exit 1
fi

if [ ! -f "$MANIFEST" ]; then
  echo "vendor.sh: missing manifest: $MANIFEST" >&2
  exit 1
fi

tag=""
base_url=""
sha256sums_name="SHA2-256SUMS"
upstream=""
expected=""

while IFS= read -r line || [ -n "$line" ]; do
  line=${line%"$CR"}
  case "$line" in
    ""|\#*) continue ;;
  esac
  key=${line%%=*}
  value=${line#*=}
  case "$key" in
    tag) tag=$value ;;
    base_url) base_url=$value ;;
    sha256sums) sha256sums_name=$value ;;
    asset)
      set -- $value
      asset_target=$1
      asset_name=$2
      asset_hash=$3
      if [ "$asset_target" = "$TARGET" ]; then
        upstream=$asset_name
        expected=$asset_hash
      fi
      ;;
  esac
done < "$MANIFEST"

if [ -z "$upstream" ] || [ -z "$expected" ]; then
  echo "vendor.sh: no asset in manifest for $TARGET" >&2
  exit 1
fi
if [ -z "$base_url" ] || [ -z "$tag" ]; then
  echo "vendor.sh: manifest is missing base_url or tag" >&2
  exit 1
fi

expected=$(echo "$expected" | tr 'A-F' 'a-f')
dest_dir="$ROOT/bin/$TARGET"
dest="$dest_dir/$upstream"
mkdir -p "$dest_dir"

sha256_file() {
  if command -v sha256sum >/dev/null 2>&1; then
    sha256sum "$1" | awk '{print $1}'
  elif command -v shasum >/dev/null 2>&1; then
    shasum -a 256 "$1" | awk '{print $1}'
  else
    echo "vendor.sh: need sha256sum or shasum" >&2
    exit 1
  fi
}

download() {
  url=$1
  out=$2
  if command -v curl >/dev/null 2>&1; then
    curl -fL --retry 3 --retry-delay 1 -A "fastpotify-ytdlp-vendor" -o "$out" "$url"
  elif command -v wget >/dev/null 2>&1; then
    wget -q --user-agent="fastpotify-ytdlp-vendor" -O "$out" "$url"
  else
    echo "vendor.sh: need curl or wget" >&2
    exit 1
  fi
}

if [ -f "$dest" ]; then
  got=$(sha256_file "$dest" | tr 'A-F' 'a-f')
  if [ "$got" = "$expected" ]; then
    echo "vendor.sh: $TARGET already has $upstream ($expected)"
    exit 0
  fi
  echo "vendor.sh: existing $dest has hash $got, expected $expected; re-downloading"
fi

work=$(mktemp -d "${TMPDIR:-/tmp}/fastpotify-ytdlp.XXXXXX")
trap 'rm -rf "$work"' EXIT INT HUP

sums_url="$base_url/$sha256sums_name"
asset_url="$base_url/$upstream"
sums_path="$work/$sha256sums_name"
part="$work/$upstream.part"

echo "vendor.sh: fetching $sha256sums_name from tag $tag"
download "$sums_url" "$sums_path"
echo "vendor.sh: fetching $upstream"
download "$asset_url" "$part"

got=$(sha256_file "$part" | tr 'A-F' 'a-f')
if [ "$got" != "$expected" ]; then
  echo "vendor.sh: hash mismatch for $upstream: got $got expected $expected" >&2
  exit 1
fi

sums_hash=""
while IFS= read -r sumline || [ -n "$sumline" ]; do
  sumline=${sumline%"$CR"}
  case "$sumline" in
    *[Ff][Ii][Ll][Ee]\ "$upstream"|*" $upstream"|*" *$upstream")
      sums_hash=$(echo "$sumline" | awk '{print $1}' | tr 'A-F' 'a-f')
      ;;
  esac
done < "$sums_path"

# Match the official "HASH  name" / "HASH *name" lines by last field.
if [ -z "$sums_hash" ]; then
  while IFS= read -r sumline || [ -n "$sumline" ]; do
    sumline=${sumline%"$CR"}
    name=${sumline##* }
    name=${name#\*}
    hash=${sumline%% *}
    if [ "$name" = "$upstream" ]; then
      sums_hash=$(echo "$hash" | tr 'A-F' 'a-f')
      break
    fi
  done < "$sums_path"
fi

if [ -z "$sums_hash" ]; then
  echo "vendor.sh: $upstream is not in $sha256sums_name" >&2
  exit 1
fi
if [ "$sums_hash" != "$expected" ]; then
  echo "vendor.sh: $sha256sums_name has $sums_hash for $upstream, manifest has $expected" >&2
  exit 1
fi
if [ "$got" != "$sums_hash" ]; then
  echo "vendor.sh: downloaded $upstream does not match $sha256sums_name" >&2
  exit 1
fi

tmp_dest="$dest.part.$$"
cp "$part" "$tmp_dest"
mv -f "$tmp_dest" "$dest"

if [ ! -f "$dest" ]; then
  echo "vendor.sh: failed to write $dest" >&2
  exit 1
fi
if [ "$REQUIRE" -eq 1 ] && [ ! -f "$dest" ]; then
  echo "vendor.sh: required asset missing: $dest" >&2
  exit 1
fi

echo "vendor.sh: wrote $dest"
echo "vendor.sh: sha256 $got"
