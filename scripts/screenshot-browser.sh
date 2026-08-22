#!/bin/sh
# Regenerates assets/screenshots/browser-form.png, without putting anybody's
# home directory in it.
#
#   scripts/screenshot-browser.sh
#
# The browser puts the address it loaded in its URL bar, which is the whole
# point of a URL bar and a problem for a screenshot: run it on the fixture where
# the fixture lives and the picture ships the path it was rendered from —
# `/Users/somebody/dev/denise/...`, naming a person and an operating system to
# everyone who reads the README.
#
# So the page is copied to `/demo` inside a container and rendered there. The
# URL bar then says `file:///demo/form.html`, which names nobody. Linux rather
# than the host for the same reason, and because a panel is what this toolkit is
# for: the font is DejaVu, which is the one `dist/install.sh` puts on the Pi.
#
# Needs Docker. Nothing else, and nothing installed on the host.
set -eu

ROOT=$(cd "$(dirname "$0")/.." && pwd)
OUT=$ROOT/assets/screenshots/browser-form.png
SIZE=${SIZE:-1100x800}
PAGE=${PAGE:-examples/browser/fixtures/form.html}

command -v docker >/dev/null 2>&1 || {
	echo "docker is needed: the point is to render somewhere that is not your home directory" >&2
	exit 1
}

printf '\n== rendering %s at %s\n' "$PAGE" "$SIZE"
docker run --rm -v "$ROOT":/work -w /work -e CARGO_TARGET_DIR=/work/target/linux \
	rust:1-slim bash -c "
set -eu
export PATH=/usr/local/cargo/bin:\$PATH
apt-get update -qq >/dev/null 2>&1
# Without a font every demo falls back to the built-in 8x8 bitmap face, and the
# screenshot would show a toolkit that cannot draw text.
apt-get install -y -qq fonts-dejavu-core >/dev/null 2>&1
mkdir -p /demo && cp '$PAGE' /demo/form.html
cargo run -q -p browser --no-default-features --features desktop,tls -- \
	--size '$SIZE' --snapshot /work/target/browser-form.ppm /demo/form.html
"

# PPM is what the demos write; PNG is what a README can show. Whichever
# converter this machine has.
if command -v magick >/dev/null 2>&1; then
	magick "$ROOT/target/browser-form.ppm" "$OUT"
elif command -v convert >/dev/null 2>&1; then
	convert "$ROOT/target/browser-form.ppm" "$OUT"
elif command -v sips >/dev/null 2>&1; then
	sips -s format png "$ROOT/target/browser-form.ppm" --out "$OUT" >/dev/null
else
	echo "no PNG converter found; the PPM is at target/browser-form.ppm" >&2
	exit 1
fi
rm -f "$ROOT/target/browser-form.ppm"

printf '   wrote %s\n' "$OUT"
printf '   check the URL bar before committing: it must not name a person\n'
