#!/usr/bin/env bash
# Install or upgrade vinoOpenEffects from a GitHub release:
#
#   curl -fsSL https://github.com/karanshukla/vinoOpenEffects/releases/latest/download/install.sh | bash
#
# Options go after `bash -s --`, e.g. `| bash -s -- --no-gui`:
#
#   vN          that release instead of the newest
#   --uninstall run the release's scripts/uninstall.sh instead, which removes
#               the install, your models and the compile cache (add --purge
#               for settings and backgrounds too)
#   -h, --help  this text
#   anything else is passed to the release's scripts/install.sh (or uninstall.sh):
#               --no-gui, --no-models, --prefix DIR (with sudo for /usr/local)
#
# It downloads the release's SHA256SUMS and binaries tarball, refuses the
# tarball unless it's listed there and matches, unpacks it into a private
# temporary directory and runs the release's own scripts/install.sh, which
# installs the prebuilt binaries (to ~/.local by default), the D-Bus and
# systemd files, onnxruntime through ovfetch (fetching ovfetch's release
# binary, hash-pinned, if it isn't installed) and the models, then restarts
# the daemon if it was running. Nothing is built. It also leaves this script
# as vinoopeneffects-upgrade, which the GUI's Check for Updates runs, and
# `vinoopeneffects-upgrade --uninstall` removes it all again.
#
# From a checkout, scripts/install.sh builds from source instead.

# Everything is in main, called on the last line: piped into bash, a download
# cut short must not run half a script.
set -euo pipefail

REPO="karanshukla/vinoOpenEffects"
ASSET="vinoopeneffects-x86_64-linux.tar.gz"

main() {
    local tag="${VINO_UPDATE_TAG:-}" script=install.sh arg
    local args=()
    for arg in "$@"; do
        case "$arg" in
            v[0-9]*) tag="$arg" ;;
            --uninstall) script=uninstall.sh ;;
            -h | --help) sed -n '2,26p' "${BASH_SOURCE[0]:-$0}" 2>/dev/null | sed 's/^# \{0,1\}//'; exit 0 ;;
            *) args+=("$arg") ;;
        esac
    done
    if [ -n "$tag" ] && ! [[ "$tag" =~ ^v[0-9]+$ ]]; then
        echo "✗ Not a release tag: '$tag'" >&2
        exit 2
    fi
    if [ "$script" = install.sh ] && [ "$(uname -m)" != x86_64 ]; then
        echo "✗ The release binaries are x86_64 only; build from a checkout on $(uname -m)" >&2
        exit 1
    fi
    local tool
    for tool in curl tar sha256sum awk; do
        command -v "$tool" >/dev/null || { echo "✗ Missing $tool: install it, then run this again" >&2; exit 1; }
    done

    # Where the assets are: a mirror, or CI's file:// fake release.
    local base
    if [ -n "${VINOOPENEFFECTS_RELEASE_BASE:-}" ]; then
        base="$VINOOPENEFFECTS_RELEASE_BASE"
    elif [ -n "$tag" ]; then
        base="https://github.com/$REPO/releases/download/$tag"
    else
        base="https://github.com/$REPO/releases/latest/download"
    fi

    local work
    work="$(mktemp -d)"
    # shellcheck disable=SC2064 # expand now: $work is local
    trap "rm -rf '$work'" EXIT

    echo "==> Downloading ${tag:-the latest release} from $base"
    local name
    for name in SHA256SUMS "$ASSET"; do
        if ! curl -fsSL --retry 5 --retry-delay 2 --retry-all-errors --connect-timeout 20 \
                -o "$work/$name" "$base/$name"; then
            echo "✗ Download failed: $base/$name" >&2
            echo "  Releases before v4 have no binaries: build from a checkout for those." >&2
            exit 1
        fi
    done
    # Listed exactly once: sha256sum -c alone would pass whatever file it named.
    if ! (cd "$work" \
            && awk -v f="$ASSET" '$2 == f || $2 == "*" f { print $1 "  " f }' SHA256SUMS > want \
            && [ "$(wc -l < want)" -eq 1 ] \
            && sha256sum -c --strict --quiet want); then
        echo "✗ Checksum verification failed for $ASSET. Nothing was installed." >&2
        exit 1
    fi
    echo "checksum verified"

    tar -xzf "$work/$ASSET" -C "$work" --no-same-owner
    # The uninstaller comes from the release too: nothing but this script is kept on disk.
    [ -x "$work/vinoopeneffects/scripts/$script" ] || { echo "✗ $ASSET has no scripts/$script" >&2; exit 1; }
    "$work/vinoopeneffects/scripts/$script" "${args[@]}" </dev/null
}

main "$@"
