#!/usr/bin/env bash
# Lets Desklink present this Linux computer to another computer as a Bluetooth keyboard and
# mouse (`desklink bluetooth`).
#
# BlueZ's classic "input" plugin owns the keyboard-and-mouse channels (L2CAP PSM 17 and 19) and
# disconnects any computer that is not a keyboard or mouse itself. This adds a systemd drop-in
# that starts bluetoothd without that plugin, so bluetoothd hands the channels to Desklink.
# Desklink itself needs no privileges.
#
# It also installs desklink-bt-qos, a helper whose only job is to ask the Bluetooth link for
# low latency (without it the host polls every 25-70 ms and the pointer moves in clumps). That
# is an HCI command, so the helper gets exactly one capability, CAP_NET_RAW, and nothing else.
#
# Cost while enabled: classic (non-BLE) Bluetooth keyboards and mice stop working on this
# computer. Bluetooth LE keyboards and mice (the "hog" plugin), audio and everything else keep
# working. `disable` restores the original service.
#
# Usage: sudo scripts/linux-bluetooth-hid.sh enable [path/to/desklink-bt-qos]|disable|status
#   The helper defaults to target/release/desklink-bt-qos (cargo build --release -p desklink-bluez).
set -euo pipefail

DROPIN_DIR=/etc/systemd/system/bluetooth.service.d
DROPIN="$DROPIN_DIR/desklink-hid.conf"
HELPER_DIR=/usr/local/libexec/desklink
HELPER="$HELPER_DIR/desklink-bt-qos"
ROOT=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)

bluetoothd_path() {
    # The packaged ExecStart, ignoring our drop-in (it only adds "-P input").
    local path
    path=$(systemctl show bluetooth -p ExecStart --value | sed -n 's/.*path=\([^ ;]*\).*/\1/p' | head -1)
    echo "${path:-/usr/lib/bluetooth/bluetoothd}"
}

status() {
    if [[ -f $DROPIN ]]; then
        echo "enabled: $DROPIN"
    else
        echo "disabled"
    fi
    if [[ -x $HELPER ]]; then
        echo "low-latency helper: $HELPER ($(getcap "$HELPER" 2>/dev/null | cut -d' ' -f2-))"
    else
        echo "low-latency helper: not installed"
    fi
    if pgrep -a bluetoothd | grep -Eq -- '(-P|--noplugin)[= ]?input'; then
        echo "bluetoothd is running without the input plugin"
    else
        echo "bluetoothd is running with the input plugin"
    fi
}

need_root() {
    [[ $EUID -eq 0 ]] || { echo "run with sudo: sudo $0 $1" >&2; exit 1; }
}

case "${1:-status}" in
    enable)
        need_root enable
        daemon=$(bluetoothd_path)
        [[ -x $daemon ]] || { echo "bluetoothd not found at $daemon" >&2; exit 1; }
        mkdir -p "$DROPIN_DIR"
        wanted=$(printf '%s\n' \
            '# Added by Desklink (scripts/linux-bluetooth-hid.sh); remove with "disable".' \
            '[Service]' 'ExecStart=' "ExecStart=$daemon -P input")
        # Restarting bluetooth drops every connection, so only do it when something changed.
        if [[ ! -f $DROPIN || $(cat "$DROPIN") != "$wanted" ]]; then
            printf '%s\n' "$wanted" > "$DROPIN"
            systemctl daemon-reload
            systemctl restart bluetooth
        fi
        helper=${2:-$ROOT/target/release/desklink-bt-qos}
        if [[ -f $helper ]]; then
            command -v setcap >/dev/null || { echo "setcap missing: install libcap2-bin" >&2; exit 1; }
            install -d -m 0755 "$HELPER_DIR"
            install -m 0755 -o root -g root "$helper" "$HELPER"
            setcap cap_net_raw+ep "$HELPER"
        else
            echo "note: $helper not found; build it with 'cargo build --release -p desklink-bluez'" \
                "and run enable again for smooth pointer motion" >&2
        fi
        status
        ;;
    disable)
        need_root disable
        rm -f "$DROPIN" "$HELPER"
        rmdir --ignore-fail-on-non-empty "$HELPER_DIR" 2>/dev/null || true
        rmdir --ignore-fail-on-non-empty "$DROPIN_DIR" 2>/dev/null || true
        systemctl daemon-reload
        systemctl restart bluetooth
        status
        ;;
    status)
        status
        ;;
    *)
        echo "usage: sudo $0 enable [helper]|disable|status" >&2
        exit 2
        ;;
esac
