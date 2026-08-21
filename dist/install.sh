#!/bin/sh
# Installs the Denise demo panel on an Alpine Raspberry Pi.
#
#   ./install.sh                     # everything, including the boot config
#   ./install.sh --no-boot-config    # binaries and services only
#   ./install.sh --demo gallery      # boot straight into one demo, no menu
#
# Run it on the Pi, from a directory holding this script, the files beside it in
# the repository's `dist/`, and the `denise-*` binaries cross-built for
# aarch64-unknown-linux-musl. `scripts/deploy-pi.sh` does the building and the
# copying; this does the installing.
#
# It is safe to run twice. Every edit checks for its own result first, every file
# it replaces is one it owns, and the two files it changes that it does not own —
# `/boot/cmdline.txt` and `/etc/inittab` — are backed up the first time and
# verified before being moved into place.
#
# What it does not do: reboot, or touch the network. The boot configuration needs
# a reboot to take effect and the script says so rather than deciding for you.
set -eu

HERE=$(cd "$(dirname "$0")" && pwd)
DEMO=""
BOOT_CONFIG=yes

while [ $# -gt 0 ]; do
	case "$1" in
	--no-boot-config) BOOT_CONFIG=no ;;
	--demo) DEMO="${2:?--demo needs a name}"; shift ;;
	-h | --help) sed -n '2,18p' "$0" | sed 's/^# \{0,1\}//'; exit 0 ;;
	*) echo "unknown argument: $1" >&2; exit 2 ;;
	esac
	shift
done

say() { printf '\n== %s\n' "$*"; }
note() { printf '   %s\n' "$*"; }

# ---------------------------------------------------------------- the checks

[ "$(id -u)" = 0 ] || { echo "run this as root: doas $0 $*" >&2; exit 1; }
[ -f /etc/alpine-release ] || {
	echo "this installer is for Alpine; see docs/raspberry-pi.md for the parts" >&2
	echo "that are not Alpine-specific" >&2
	exit 1
}
[ -d /boot/overlays ] || note "no /boot/overlays: not a Raspberry Pi boot partition?"

MISSING=""
for bin in denise-gallery denise-launcher denise-splash; do
	[ -f "$HERE/$bin" ] || MISSING="$MISSING $bin"
done
[ -z "$MISSING" ] || {
	echo "missing binaries:$MISSING" >&2
	echo "cross-build them first -- scripts/deploy-pi.sh does it for you" >&2
	exit 1
}

say "installing onto Alpine $(cat /etc/alpine-release), kernel $(uname -r)"

# ------------------------------------------------------------------- the font

# Stock Alpine has no fonts at all, and without one every demo falls back to the
# built-in 8x8 bitmap face -- which works, and looks like 1985.
if [ ! -d /usr/share/fonts/dejavu ]; then
	say "installing font-dejavu"
	apk add --quiet font-dejavu
else
	note "font-dejavu is already here"
fi

# --------------------------------------------------------------- the binaries

say "installing binaries into /usr/local/bin"
for bin in "$HERE"/denise-*; do
	name=$(basename "$bin")
	case "$name" in *.txt | *.sh) continue ;; esac
	# `install` and not `cp`: the running panel holds its binary open, and
	# writing through it fails with ETXTBSY where replacing the file does not.
	install -m 755 -o root -g root "$bin" "/usr/local/bin/$name"
	note "$name"
done
for helper in denise-run denise-console denise-demo; do
	[ -f "$HERE/$helper" ] || continue
	install -m 755 -o root -g root "$HERE/$helper" "/usr/local/bin/$helper"
	note "$helper"
done

# ------------------------------------------------------------------ the files

say "installing services and configuration"
install -m 755 -o root -g root "$HERE/init.d/denise" /etc/init.d/denise
install -m 755 -o root -g root "$HERE/init.d/denise-splash" /etc/init.d/denise-splash
install -m 644 -o root -g root "$HERE/denise-exit-hint.txt" /etc/denise-exit-hint.txt
install -m 644 -o root -g root "$HERE/motd" /etc/motd

# conf.d is the operator's file once it exists: replacing it on an upgrade would
# silently undo whichever demo they chose.
if [ -f /etc/conf.d/denise ]; then
	note "/etc/conf.d/denise kept (yours)"
else
	install -m 644 -o root -g root "$HERE/conf.d/denise" /etc/conf.d/denise
	note "/etc/conf.d/denise"
fi
if [ -n "$DEMO" ]; then
	sed -i "s|^demo=.*|demo=\"$DEMO\"|" /etc/conf.d/denise
	note "boot demo set to $DEMO"
fi

rc-update add denise-splash sysinit >/dev/null 2>&1 || true
rc-update add denise default >/dev/null 2>&1 || true
note "denise-splash in sysinit, denise in default"

if [ "$BOOT_CONFIG" = no ]; then
	say "done -- boot configuration skipped"
	note "start it now with: rc-service denise start"
	exit 0
fi

# ------------------------------------------------------------- the boot config

say "boot configuration"

# Full KMS. Without it there is no /dev/dri, the demos fall back to a tearing
# single-buffered fbdev, and denise-video has no decoder at all.
touch /boot/usercfg.txt
add_usercfg() {
	grep -q "^$1" /boot/usercfg.txt || {
		printf '\n# %s\n%s\n' "$2" "$1" >>/boot/usercfg.txt
		note "usercfg.txt: $1"
	}
}
add_usercfg "dtoverlay=vc4-kms-v3d" "Real modesetting: page flips instead of a tearing single buffer."
add_usercfg "gpu_mem=128" "The V4L2 decoder's buffers are firmware-side; 64 MB is not enough for 1080p."
add_usercfg "disable_splash=1" "No rainbow test card on the way to the splash."
add_usercfg "disable_overscan=1" "So the firmware framebuffer and vc4's agree on the size."

# The hardware decoder. Raspberry Pi OS binds this from the device tree and
# Alpine does not, so it has to be named.
grep -q '^bcm2835-codec$' /etc/modules 2>/dev/null || {
	echo bcm2835-codec >>/etc/modules
	note "/etc/modules: bcm2835-codec"
}

# Kernel messages and OpenRC's chatter to a VT nobody is looking at, so the
# screen stays clean from power-on. tty8 has no getty on it and never will.
if ! grep -q 'console=tty8' /boot/cmdline.txt; then
	[ -f /boot/cmdline.txt.before-denise ] || cp -a /boot/cmdline.txt /boot/cmdline.txt.before-denise
	# `console=tty8` only. `vt.global_cursor_default=0` also hides the cursor,
	# and does it on *every* VT — which is why a login prompt reached with Alt+F2
	# came up without one and looked like a hung machine. The splash hides it on
	# tty1 instead, where the panel is, and leaves the consoles alone.
	printf '%s\n' "$(cat /boot/cmdline.txt) console=tty8" >/boot/cmdline.txt.new
	# One line, always. A second line here is a kernel that boots without half
	# its parameters, and the recovery is a card reader.
	if [ "$(wc -l </boot/cmdline.txt.new | tr -d ' ')" = 1 ]; then
		mv /boot/cmdline.txt.new /boot/cmdline.txt
		note "cmdline.txt: console=tty8 (backup: cmdline.txt.before-denise)"
	else
		rm -f /boot/cmdline.txt.new
		echo "refusing to write a multi-line cmdline.txt; left it alone" >&2
	fi
else
	note "cmdline.txt already sends the console to tty8"
fi

# tty1 is the panel's, so nothing prints a login prompt over it. The rest run
# through `denise-console`, which is a getty that brings the panel back when the
# session ends -- so "Exit to console" is a visit rather than a one-way door.
if ! grep -q 'denise-console' /etc/inittab; then
	[ -f /etc/inittab.before-denise ] || cp -a /etc/inittab /etc/inittab.before-denise
	sed -i \
		-e 's|^tty1::respawn:/sbin/getty.*|# tty1 is the panel: no getty, so nothing prints over it\n#&|' \
		-e 's|^\(tty[2-6]\)::respawn:/sbin/getty [0-9]* \(tty[2-6]\)$|\1::respawn:/usr/local/bin/denise-console \2|' \
		/etc/inittab
	note "inittab: tty1 freed, tty2-6 through denise-console (backup: inittab.before-denise)"
	# init re-reads inittab on SIGHUP, so the consoles change hands now rather
	# than at the next boot. Nothing else about this needs one.
	kill -HUP 1 2>/dev/null || true
else
	note "inittab already goes through denise-console"
fi

say "done"
note "The boot configuration needs a reboot. Until then:"
note "  rc-service denise start     puts the panel up now"
note "  rc-service denise stop      takes it down"
note "  denise-demo list            runs one demo by hand"
