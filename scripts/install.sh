#!/usr/bin/env bash
# Build and install vinoOpenEffects from this checkout.
#
#   scripts/install.sh                    # to ~/.local, no root needed
#   sudo scripts/install.sh --prefix /usr/local   # after building as yourself, see below
#   scripts/install.sh --no-gui           # daemon and CLI only
#   scripts/install.sh --build-runtime    # let ovfetch compile a newer OpenVINO build
#
# Installs binaries, D-Bus interfaces and activation file, the systemd user
# unit, desktop entry and icons, fetches the sha256-verified ONNX models, and
# has ovfetch (>= 0.5.0, the only source of onnxruntime) install the runtime
# into lib/vinoopeneffects/ort, keeping a verified ovfetch install it won't replace.
#
# As root it never builds (that would leave a root-owned target/): build first
# with `cargo build --release --locked -p vinoopeneffects-daemon -p
# vinoopeneffects-ctl -p vinoopeneffects`. Models then go to share/ under the
# prefix, and the user-session reloads run as the sudo user.
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$repo_root"

args="$*"
prefix="$HOME/.local"
gui=1
models=1
ovfetch_args=()
ovfetch_min=0.5.0
while [ $# -gt 0 ]; do
    case "$1" in
        --prefix) prefix="${2:?--prefix needs a directory}"; shift 2 ;;
        --no-gui) gui=0; shift ;;
        --no-models) models=0; shift ;;
        --build-runtime) ovfetch_args+=(--build); shift ;;
        -h | --help) sed -n '2,17p' "$0" | sed 's/^# \{0,1\}//'; exit 0 ;;
        *) echo "unknown option: $1 (try --help)" >&2; exit 2 ;;
    esac
done

die() { echo "install: $*" >&2; exit 1; }

as_root=0
[ "$(id -u)" = 0 ] && as_root=1
user_home=""
if [ "$as_root" = 1 ] && [ -n "${SUDO_USER:-}" ] && [ "$SUDO_USER" != root ]; then
    user_home="$(getent passwd "$SUDO_USER" | cut -d: -f6)"
fi

ovfetch="$(command -v ovfetch || true)"
# sudo's secure_path drops ~/.cargo/bin, where `cargo install` put it.
if [ -z "$ovfetch" ] && [ -n "$user_home" ] && [ -x "$user_home/.cargo/bin/ovfetch" ]; then
    ovfetch="$user_home/.cargo/bin/ovfetch"
fi
if [ -z "$ovfetch" ] && [ "$as_root" = 1 ]; then
    die "ovfetch is not on root's PATH: sudo env PATH=\"\$PATH\" $0 $args"
fi
[ -n "$ovfetch" ] ||
    die "ovfetch installs onnxruntime and is not on PATH: cargo install ovfetch --locked"
have="$("$ovfetch" --version | awk '{print $2}')"
if [ "$(printf '%s\n%s\n' "$ovfetch_min" "$have" | sort -V | head -1)" != "$ovfetch_min" ]; then
    die "ovfetch $have is older than $ovfetch_min: cargo install ovfetch --locked"
fi

bindir="$prefix/bin"
datadir="$prefix/share"
target="${CARGO_TARGET_DIR:-$repo_root/target}/release"
app_id=io.github.karanshukla.vinoopeneffects

bins=(vinoopeneffects-daemon vinoopeneffects-ctl)
build=(-p vinoopeneffects-daemon -p vinoopeneffects-ctl)
if [ "$gui" = 1 ]; then
    bins+=(vinoopeneffects)
    build+=(-p vinoopeneffects)
fi

if [ "$as_root" = 1 ]; then
    for bin in "${bins[@]}"; do
        [ -x "$target/$bin" ] ||
            die "$target/$bin is missing; as root this script doesn't build. As yourself: cargo build --release --locked ${build[*]}"
    done
    stale="$(find shared core daemon cli gui Cargo.lock -newer "$target/vinoopeneffects-daemon" \
        \( -name '*.rs' -o -name '*.toml' -o -name '*.xml' -o -name Cargo.lock \) -print -quit 2>/dev/null || true)"
    [ -z "$stale" ] ||
        echo "warning: $stale is newer than the release build; rebuild as yourself first" >&2
else
    echo "==> Building (release)"
    cargo build --release --locked "${build[@]}"
fi

ortdir="$prefix/lib/vinoopeneffects/ort"
echo "==> onnxruntime (ovfetch)"
if ovfetch_out="$("$ovfetch" install --prefix "$ortdir" "${ovfetch_args[@]}" 2>&1)"; then
    echo "$ovfetch_out"
elif "$ovfetch" verify --prefix "$ortdir" >/dev/null 2>&1; then
    echo "keeping the verified ovfetch install in $ortdir"
    echo "  ovfetch: $(echo "$ovfetch_out" | sed -n 's/^Error: //p' | tail -1)"
else
    echo "$ovfetch_out" >&2
    die "ovfetch did not install onnxruntime into $ortdir, and there is no verified ovfetch install to keep"
fi

echo "==> Installing to $prefix"
install -d "$bindir"
for bin in "${bins[@]}"; do
    install -m 0755 "$target/$bin" "$bindir/$bin"
done

install -d "$datadir/dbus-1/interfaces"
install -m 0644 data/dbus/*.xml "$datadir/dbus-1/interfaces/"

install -d "$datadir/dbus-1/services" "$datadir/systemd/user"
sed "s|@bindir@|$bindir|g" "data/dbus-services/$app_id.Daemon.service.in" \
    >"$datadir/dbus-1/services/$app_id.Daemon.service"
sed "s|@bindir@|$bindir|g" data/systemd/vinoopeneffects-daemon.service.in \
    >"$datadir/systemd/user/vinoopeneffects-daemon.service"

if [ "$gui" = 1 ]; then
    install -d "$datadir/applications" \
        "$datadir/icons/hicolor/scalable/apps" "$datadir/icons/hicolor/symbolic/apps"
    install -m 0644 "data/applications/$app_id.desktop" "$datadir/applications/"
    install -m 0644 "data/icons/$app_id.svg" "$datadir/icons/hicolor/scalable/apps/"
    install -m 0644 "data/icons/$app_id-symbolic.svg" "$datadir/icons/hicolor/symbolic/apps/"
fi

if [ "$models" = 1 ]; then
    echo "==> Fetching models"
    # As root, HOME is /root; the daemon also searches share/ under its own prefix.
    if [ "$as_root" = 1 ]; then
        VINOOPENEFFECTS_MODELS_DEST="$datadir/vinoopeneffects/models" ./scripts/fetch-models.sh
    else
        ./scripts/fetch-models.sh
    fi
fi

# As root, --user would reach root's session (or none), not the one that runs the daemon.
user_session() {
    if [ "$as_root" = 0 ]; then
        "$@"
    elif [ -n "$user_home" ]; then
        uid="$(id -u "$SUDO_USER")"
        sudo -u "$SUDO_USER" XDG_RUNTIME_DIR="/run/user/$uid" \
            DBUS_SESSION_BUS_ADDRESS="unix:path=/run/user/$uid/bus" "$@"
    else
        return 1
    fi
}

echo "==> Refreshing caches"
# The bus reads new activation files only on reload; otherwise calls fail "name not activatable" until next login.
if command -v busctl >/dev/null 2>&1; then
    user_session busctl --user call org.freedesktop.DBus /org/freedesktop/DBus \
        org.freedesktop.DBus ReloadConfig >/dev/null 2>&1 || true
fi
if command -v systemctl >/dev/null 2>&1; then
    user_session systemctl --user daemon-reload >/dev/null 2>&1 || true
fi
if [ "$gui" = 1 ]; then
    if command -v gtk-update-icon-cache >/dev/null 2>&1; then
        gtk-update-icon-cache -q -t -f "$datadir/icons/hicolor" || true
    fi
    if command -v update-desktop-database >/dev/null 2>&1; then
        update-desktop-database -q "$datadir/applications" || true
    fi
fi

case ":$PATH:" in
    *":$bindir:"*) ;;
    *) echo "note: $bindir is not on PATH" >&2 ;;
esac
case ":${XDG_DATA_DIRS:-/usr/local/share:/usr/share}:" in
    *":$datadir:"*) ;;
    *) [ "$prefix" = "$HOME/.local" ] ||
        echo "note: $datadir is not in XDG_DATA_DIRS; D-Bus activation and the unit may not be found" >&2 ;;
esac

if [ "$as_root" = 1 ] && [ -z "$user_home" ]; then
    echo "note: run as root without sudo, so no user session was reloaded; log in again or run" >&2
    echo "  systemctl --user daemon-reload as the user who will run the daemon" >&2
fi

echo "==> Done. Try: vinoopeneffects-ctl status"
