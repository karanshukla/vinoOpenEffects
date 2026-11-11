#!/usr/bin/env bash
# Remove what scripts/install.sh installed.
#
#   scripts/uninstall.sh                  # from ~/.local
#   sudo scripts/uninstall.sh --prefix /usr/local
#   scripts/uninstall.sh --purge          # also config and backgrounds
#
# It also deletes your downloaded models and the NPU compile cache (under sudo,
# the sudo user's); --purge is only for the settings and backgrounds you made.
# A release install runs this with `vinoopeneffects-upgrade --uninstall`.
set -euo pipefail

prefix="$HOME/.local"
purge=0
while [ $# -gt 0 ]; do
    case "$1" in
        --prefix) prefix="${2:?--prefix needs a directory}"; shift 2 ;;
        --purge) purge=1; shift ;;
        -h | --help) sed -n '2,10p' "$0" | sed 's/^# \{0,1\}//'; exit 0 ;;
        *) echo "unknown option: $1 (try --help)" >&2; exit 2 ;;
    esac
done

bindir="$prefix/bin"
datadir="$prefix/share"
app_id=io.github.karanshukla.vinoopeneffects

as_root=0
[ "$(id -u)" = 0 ] && as_root=1

# Whose models, config and backgrounds: under sudo, the user who ran it.
user_data="${XDG_DATA_HOME:-$HOME/.local/share}"
user_config="${XDG_CONFIG_HOME:-$HOME/.config}"
user_cache="${XDG_CACHE_HOME:-$HOME/.cache}"
if [ "$as_root" = 1 ] && [ -n "${SUDO_USER:-}" ] && [ "$SUDO_USER" != root ]; then
    user_home="$(getent passwd "$SUDO_USER" | cut -d: -f6)"
    if [ -n "$user_home" ]; then
        user_data="$user_home/.local/share"
        user_config="$user_home/.config"
        user_cache="$user_home/.cache"
    fi
fi

# As root, --user would reach root's session, not the one running the daemon.
user_session() {
    if [ "$as_root" = 0 ]; then
        "$@"
    elif [ -n "${SUDO_USER:-}" ] && [ "$SUDO_USER" != root ]; then
        uid="$(id -u "$SUDO_USER")"
        sudo -u "$SUDO_USER" XDG_RUNTIME_DIR="/run/user/$uid" \
            DBUS_SESSION_BUS_ADDRESS="unix:path=/run/user/$uid/bus" "$@"
    else
        return 1
    fi
}

if command -v systemctl >/dev/null 2>&1; then
    user_session systemctl --user disable --now vinoopeneffects-daemon.service >/dev/null 2>&1 || true
fi

rm -f "$bindir/vinoopeneffects-daemon" "$bindir/vinoopeneffects-ctl" "$bindir/vinoopeneffects" "$bindir/vinoopeneffects-upgrade"
rm -f "$datadir"/dbus-1/interfaces/"$app_id".*.xml
rm -f "$datadir/dbus-1/services/$app_id.Daemon.service"
rm -f "$datadir/systemd/user/vinoopeneffects-daemon.service"
rm -f "$datadir/applications/$app_id.desktop"
rm -f "$datadir/icons/hicolor/scalable/apps/$app_id.svg"
rm -f "$datadir/icons/hicolor/symbolic/apps/$app_id-symbolic.svg"
rm -rf "$prefix/lib/vinoopeneffects"
rmdir "$prefix/lib" 2>/dev/null || true
# A system-wide install put the models under the prefix.
if [ "$as_root" = 1 ]; then
    rm -rf "$datadir/vinoopeneffects/models"
    rmdir "$datadir/vinoopeneffects" 2>/dev/null || true
fi

# The models and what's compiled from them; the backgrounds beside them stay.
rm -rf "$user_data/vinoopeneffects/models" "$user_cache/vinoopeneffects"
rmdir "$user_data/vinoopeneffects" 2>/dev/null || true

if [ "$purge" = 1 ]; then
    rm -rf "$user_data/vinoopeneffects" "$user_config/vinoopeneffects"
fi

if command -v busctl >/dev/null 2>&1; then
    user_session busctl --user call org.freedesktop.DBus /org/freedesktop/DBus \
        org.freedesktop.DBus ReloadConfig >/dev/null 2>&1 || true
fi
if command -v systemctl >/dev/null 2>&1; then
    user_session systemctl --user daemon-reload >/dev/null 2>&1 || true
fi
if command -v gtk-update-icon-cache >/dev/null 2>&1; then
    gtk-update-icon-cache -q -t -f "$datadir/icons/hicolor" 2>/dev/null || true
fi

echo "Removed vinoOpenEffects from $prefix, and the models in $user_data/vinoopeneffects/models."
if [ "$purge" = 0 ] && { [ -e "$user_config/vinoopeneffects" ] || [ -e "$user_data/vinoopeneffects" ]; }; then
    echo "Kept your settings and backgrounds; --purge deletes those too."
fi
