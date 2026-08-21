#!/bin/sh
# Cross-builds the demo panel and installs it on a Raspberry Pi over ssh.
#
#   scripts/deploy-pi.sh rpi3b
#   scripts/deploy-pi.sh rpi3b --no-boot-config
#   DENISE_TLS=0 scripts/deploy-pi.sh rpi3b        # browser without https
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
note() { printf '   %s\n' "$*"; }

# ------------------------------------------------------------ https on the Pi
#
# The browser's `tls` feature pulls rustls, rustls pulls ring, and ring's build
# compiles C — the one thing a toolchain-free cross story has no compiler for.
# It very nearly does, though. ring builds its C with `-nostdlibinc` and wants
# no libc headers at all, so any clang can aim at aarch64-musl, and the archiver
# it needs ships with rustup. Three variables, and the panel speaks https with
# nothing installed that a Rust developer did not already have.
#
# This matters more than a feature flag suggests: without it the browser demo is
# barely a demo. Its own welcome page links to https, the URL bar assumes https
# for anything typed bare, and so does every site worth opening.
#
# `DENISE_TLS=0 scripts/deploy-pi.sh ...` opts out.
tls_env() {
	if [ "${DENISE_TLS-1}" = 0 ]; then
		note "DENISE_TLS=0: the browser is built without https"
		return 1
	fi

	# A real cross toolchain, where somebody has installed one, needs no help.
	if command -v aarch64-linux-musl-gcc >/dev/null 2>&1; then
		CC_aarch64_unknown_linux_musl=aarch64-linux-musl-gcc
		AR_aarch64_unknown_linux_musl=aarch64-linux-musl-ar
		export CC_aarch64_unknown_linux_musl AR_aarch64_unknown_linux_musl
		return 0
	fi

	if ! command -v clang >/dev/null 2>&1; then
		note "no clang and no aarch64-linux-musl-gcc: the browser loses https"
		return 1
	fi

	# The system `ar` on a Mac writes an archive shape rust-lld reads as empty,
	# and the link then fails on every symbol ring's C provides. rustup has a
	# working one, one component away.
	LLVM_AR="$(rustc --print sysroot)/lib/rustlib/$(rustc -vV | sed -n 's/^host: //p')/bin/llvm-ar"
	if [ ! -x "$LLVM_AR" ] && command -v rustup >/dev/null 2>&1; then
		note "adding the llvm-tools component, for its archiver"
		rustup component add llvm-tools >/dev/null 2>&1 || true
	fi
	if [ ! -x "$LLVM_AR" ]; then
		note "no llvm-ar: run \`rustup component add llvm-tools\` for https"
		return 1
	fi

	CC_aarch64_unknown_linux_musl=clang
	# `-nostdlibinc` is ring's own doing. `-U__musl__` is ours: Apple's clang
	# hands <stddef.h> to a musl system header when it sees a musl target, and a
	# cross build is precisely the case with no copy of that header. Undefining
	# the macro puts clang back on its own. Everywhere else it is not set, and
	# undefining it costs nothing.
	CFLAGS_aarch64_unknown_linux_musl="--target=$TARGET -U__musl__"
	AR_aarch64_unknown_linux_musl="$LLVM_AR"
	export CC_aarch64_unknown_linux_musl CFLAGS_aarch64_unknown_linux_musl
	export AR_aarch64_unknown_linux_musl
	return 0
}

if tls_env; then
	BROWSER_FEATURES=kiosk,tls
else
	BROWSER_FEATURES=kiosk
fi

say "building for $TARGET"
note "browser: $BROWSER_FEATURES"
# Two shapes of demo. `panel`, `kiosk`, `launcher` and `splash` are Linux-only
# and build as they are; the rest choose a backend at compile time and have to be
# asked for the kiosk one, because no runtime probe can tell a kiosk Pi from a
# Pi running a desktop.
for crate in launcher splash panel kiosk; do
	cargo build --release --target "$TARGET" -p "$crate" --manifest-path "$ROOT/Cargo.toml"
done
for crate in gallery hello table-editor; do
	cargo build --release --target "$TARGET" -p "$crate" \
		--no-default-features --features kiosk --manifest-path "$ROOT/Cargo.toml"
done
cargo build --release --target "$TARGET" -p browser \
	--no-default-features --features "$BROWSER_FEATURES" --manifest-path "$ROOT/Cargo.toml"
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
