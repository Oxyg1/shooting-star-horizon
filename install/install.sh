#!/usr/bin/env bash
# The Shooting Star (Demo) for Horizon: installer for Linux and macOS.
#
#   curl -fsSL https://raw.githubusercontent.com/Oxyg1/shooting-star-horizon/main/install/install.sh | bash
#
# 1. Downloads the original mod (rimuru.dev, All Rights Reserved) from CurseForge, as you would:
#    this repository carries none of its files.
# 2. Builds the content mod from it (items, blocks, sounds, the fx shaders, textures, the films'
#    camera tables) into your Horizon game folder and turns it on.
# 3. Installs the native mod (the port's Rust code) from GitHub with the Horizon Launcher and builds
#    the game with it.
#
# Options: --game-dir DIR (default: $HORIZON_HOME, else Horizon's data folder),
#          --launcher PATH (default: horizon-launcher on PATH or in the usual places),
#          --local (build the native mod from this checkout instead of GitHub),
#          --content-only (skip the native mod), --no-build (install, but build later).
set -euo pipefail

REPO="${SHOOTING_STAR_REPO:-Oxyg1/shooting-star-horizon}"
REF="${SHOOTING_STAR_REF:-main}"
MOD_ID="shooting_star_demo"
# The original's files on CurseForge (project 1719195), checked by SHA-256.
JAR_152_URL="https://www.curseforge.com/api/v1/mods/1719195/files/9438887/download"
JAR_152_SHA="90d3c145ea00a3a5c4119b51f42b06cd0569c83733d2f3a5398579e77f4d703a"
JAR_152_INNER="META-INF/jars/the-shooting-star-demo-1.5.2-demo+mc26.3.jar"
JAR_144_URL="https://www.curseforge.com/api/v1/mods/1719195/files/9231726/download"
JAR_144_SHA="1c361965467e66c0050609f5f9ce53ec4a70677ec74dc5f119da79bdc46efa1a"
JAR_144_INNER="META-INF/jars/the-shooting-star-demo-1.4.4-demo+mc26.3.jar"

say() { printf '\033[1;35m✦\033[0m %s\n' "$*"; }
die() { printf '\033[1;31m✗\033[0m %s\n' "$*" >&2; exit 1; }

GAME_DIR="" LAUNCHER="" LOCAL=0 CONTENT_ONLY=0 BUILD=1
while [ $# -gt 0 ]; do
  case "$1" in
    --game-dir) GAME_DIR="$2"; shift 2 ;;
    --launcher) LAUNCHER="$2"; shift 2 ;;
    --local) LOCAL=1; shift ;;
    --content-only) CONTENT_ONLY=1; shift ;;
    --no-build) BUILD=0; shift ;;
    -h|--help) sed -n '2,20p' "$0"; exit 0 ;;
    *) die "unknown option $1 (see --help)" ;;
  esac
done

for tool in curl unzip; do command -v "$tool" >/dev/null || die "$tool is needed: install it and run this again"; done
sha256() { if command -v sha256sum >/dev/null; then sha256sum "$1" | cut -d' ' -f1; else shasum -a 256 "$1" | cut -d' ' -f1; fi; }

# The game folder, as the launcher's game_home().
if [ -z "$GAME_DIR" ]; then
  if [ -n "${HORIZON_HOME:-}" ]; then GAME_DIR="$HORIZON_HOME"
  elif [ "$(uname)" = Darwin ]; then GAME_DIR="$HOME/Library/Application Support/horizon"
  else GAME_DIR="${XDG_DATA_HOME:-$HOME/.local/share}/horizon"; fi
fi

# This checkout, or the repository downloaded.
WORK="$(mktemp -d "${TMPDIR:-/tmp}/shooting-star.XXXXXX")"
trap 'rm -rf "$WORK"' EXIT
SRC=""
if [ -n "${BASH_SOURCE[0]:-}" ] && [ -f "$(dirname "${BASH_SOURCE[0]}")/files.tsv" ]; then
  SRC="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
else
  say "Downloading $REPO ($REF)"
  curl -fsSL "https://codeload.github.com/$REPO/tar.gz/$REF" | tar -xz -C "$WORK"
  SRC="$(find "$WORK" -mindepth 1 -maxdepth 1 -type d | head -n1)"
fi

# The original's jars, kept between runs.
CACHE="${XDG_CACHE_HOME:-$HOME/.cache}/shooting-star-horizon"
mkdir -p "$CACHE"
fetch() { # url sha file
  if [ -f "$3" ] && [ "$(sha256 "$3")" = "$2" ]; then return; fi
  say "Downloading $(basename "$3") from CurseForge"
  curl -fL --progress-bar -o "$3.part" "$1"
  [ "$(sha256 "$3.part")" = "$2" ] || die "$(basename "$3"): not the file this port was made for (checksum)"
  mv "$3.part" "$3"
}
fetch "$JAR_152_URL" "$JAR_152_SHA" "$CACHE/the-shooting-star-demo-1.5.2-fabric.jar"
fetch "$JAR_144_URL" "$JAR_144_SHA" "$CACHE/the-shooting-star-demo-1.4.4-fabric.jar"
unzip -p "$CACHE/the-shooting-star-demo-1.5.2-fabric.jar" "$JAR_152_INNER" > "$WORK/152.jar"
unzip -p "$CACHE/the-shooting-star-demo-1.4.4-fabric.jar" "$JAR_144_INNER" > "$WORK/144.jar"

# The content mod: the port's own files, then the original's.
say "Building the content mod"
OUT="$WORK/content"
cp -R "$SRC/content" "$OUT"
count=0
while IFS=$'\t' read -r dest jar from; do
  case "$dest" in ''|'#'*) continue ;; esac
  mkdir -p "$OUT/$(dirname "$dest")"
  unzip -p "$WORK/${jar//./}.jar" "$from" > "$OUT/$dest" || die "$from is missing from the $jar jar"
  count=$((count + 1))
done < "$SRC/install/files.tsv"
say "  $count of the original's files"

# The films' camera tables: the original's path classes, run once (Java 25, as Minecraft 26.3's).
JAVA=""
find_java() {
  for j in "${JAVA_HOME:-}/bin/java" "$(command -v java || true)"; do
    if [ -x "$j" ] && "$j" -version 2>&1 | grep -Eq 'version "(2[5-9]|[3-9][0-9])'; then JAVA="$j"; return; fi
  done
  if [ -n "$LAUNCHER" ]; then
    local home
    home="$("$LAUNCHER" --json java install 25 2>/dev/null | sed -n 's/.*"home"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/p' | head -n1)"
    if [ -n "$home" ] && [ -x "$home/bin/java" ]; then JAVA="$home/bin/java"; fi
  fi
}
if [ -z "$LAUNCHER" ]; then
  for l in "$(command -v horizon-launcher || true)" "$HOME/.local/bin/horizon-launcher" "/Applications/Horizon Launcher.app/Contents/MacOS/horizon-launcher"; do
    if [ -n "$l" ] && [ -x "$l" ]; then LAUNCHER="$l"; break; fi
  done
fi
find_java
if [ -n "$JAVA" ]; then
  say "Recording the films' cameras (Java: $JAVA)"
  "$JAVA" -cp "$SRC/tools/films/classes:$WORK/152.jar" Films "$OUT/data/$MOD_ID/films" >/dev/null 2>"$WORK/films.log" \
    || { cat "$WORK/films.log" >&2; die "the films' tables could not be made"; }
else
  say "No Java 25 found: the films (the cutscenes in space) stay off. Install Horizon Launcher, or Java 25, and run this again."
fi

# Into the game folder, turned on.
TARGET="$GAME_DIR/horizon-mods/$MOD_ID"
mkdir -p "$GAME_DIR/horizon-mods"
rm -rf "$TARGET.old"
[ -d "$TARGET" ] && mv "$TARGET" "$TARGET.old"
cp -R "$OUT" "$TARGET"
rm -rf "$TARGET.old"
LIST="$GAME_DIR/horizon-mods.txt"
touch "$LIST"
grep -qx "mod.$MOD_ID:true" "$LIST" || { sed -i.bak "/^mod\.$MOD_ID:/d" "$LIST" 2>/dev/null || true; rm -f "$LIST.bak"; echo "mod.$MOD_ID:true" >> "$LIST"; }
say "Content mod installed: $TARGET"

[ "$CONTENT_ONLY" = 1 ] && { say "Done (content only)."; exit 0; }
[ -n "$LAUNCHER" ] || die "Horizon Launcher was not found: pass --launcher /path/to/horizon-launcher"

# The native mod: the port's code, built into the game by the launcher.
if [ "$LOCAL" = 1 ]; then
  say "Building the native mod from $SRC/native"
  [ "$BUILD" = 1 ] && "$LAUNCHER" native-build "$SRC/native" --trust-native-code
else
  say "Installing the native mod from github.com/$REPO"
  "$LAUNCHER" mods add-github "https://github.com/$REPO" --trust-native-code
  [ "$BUILD" = 1 ] && "$LAUNCHER" native-build --trust-native-code
fi
say "Done. Open Horizon Launcher and press Play: the Stellar Remote is in the creative menu."
