#!/bin/sh
# Cross-builds the demo panel and installs it on a Raspberry Pi over ssh.
#
#   scripts/deploy-pi.sh rpi3b
#   scripts/deploy-pi.sh rpi3b --no-boot-config
#
# The host is anything ssh understands: a name from `~/.ssh/config`, `user@host`,
# an address. Everything after it is passed to `dist/install.sh` on the far end.
#
# The Pi needs no toolchain. musl links statically with the linker rustc already
# ships, so the binaries that land there depend on nothing but the kernel — which
# is the whole reason a 900 MHz board never has to compile anything.
set -eu

HOST="${1:?usage: deploy-pi.sh <ssh-host> [install.sh arguments...]}"
shift

ROOT=$(cd "$(dirname "$0")/.." && pwd)
TARGET=aarch64-unknown-linux-musl
OUT="$ROOT/target/$TARGET/release"
STAGE=$(mktemp -d)
trap 'rm -rf "$STAGE"' EXIT

say() { printf '\n== %s\n' "$*"; }

say "building for $TARGET"
# Two shapes of demo. `panel`, `kiosk`, `launcher` and `splash` are Linux-only
# and build as they are; the rest choose a backend at compile time and have to be
# asked for the kiosk one, because no runtime probe can tell a kiosk Pi from a
# Pi running a desktop.
for crate in launcher splash panel kiosk; do
	cargo build --release --target "$TARGET" -p "$crate" --manifest-path "$ROOT/Cargo.toml"
done
for crate in gallery hello table-editor browser; do
	cargo build --release --target "$TARGET" -p "$crate" \
		--no-default-features --features kiosk --manifest-path "$ROOT/Cargo.toml"
done
cargo build --release --target "$TARGET" -p denise-video --example player \
	--manifest-path "$ROOT/Cargo.toml"
cargo build --release --target "$TARGET" -p denise-video --example probe \
	--manifest-path "$ROOT/Cargo.toml"

say "staging"
# Renamed on the way, so everything on the Pi shares one prefix: that prefix is
# what the launcher scans for, and what the service's stop() kills.
cp "$OUT/launcher" "$STAGE/denise-launcher"
cp "$OUT/splash" "$STAGE/denise-splash"
cp "$OUT/panel" "$STAGE/denise-panel"
cp "$OUT/kiosk" "$STAGE/denise-kiosk"
cp "$OUT/gallery" "$STAGE/denise-gallery"
cp "$OUT/hello" "$STAGE/denise-hello"
cp "$OUT/table-editor" "$STAGE/denise-table-editor"
cp "$OUT/browser" "$STAGE/denise-browser"
cp "$OUT/examples/player" "$STAGE/denise-video-player"
cp "$OUT/examples/probe" "$STAGE/denise-video-probe"
cp -R "$ROOT/dist/." "$STAGE/"
chmod +x "$STAGE"/*.sh "$STAGE"/denise-* 2>/dev/null || true
du -sh "$STAGE" | sed 's/^/   /'

say "copying to $HOST"
ssh "$HOST" 'rm -rf /tmp/denise-install && mkdir -p /tmp/denise-install'
# `tar` over `scp -r` for one round trip rather than one per file, on a link that
# is often wifi.
tar -C "$STAGE" -cf - . | ssh "$HOST" 'tar -C /tmp/denise-install -xf -'

say "installing"
# doas, then sudo: Alpine has doas and Raspberry Pi OS has sudo, and asking for
# the wrong one is a confusing way to fail.
ssh -t "$HOST" "if command -v doas >/dev/null; then doas sh /tmp/denise-install/install.sh $*; \
	else sudo sh /tmp/denise-install/install.sh $*; fi"
ssh "$HOST" 'rm -rf /tmp/denise-install'
