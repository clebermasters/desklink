#!/usr/bin/env sh
# Pairs this computer and an Android phone in both directions over adb, in one step.
#
#   scripts/android-pair.sh [-s SERIAL] [--edge left|right|top|bottom] [--address IP:PORT]
#
# --edge     where the phone sits relative to this computer (default: right). With the layout
#            editor you can rearrange it later.
# --address  how the phone reaches this computer (default: guessed from the default route).
# Without adb, copy the pairing codes by hand instead: `desklink pairing-code` on the computer,
# and "This phone → Copy/Share" in the app.
set -eu
PACKAGE=com.desklink.android
ADB=${ADB:-adb}
DESKLINK=${DESKLINK:-desklink}
EDGE=right
ADDRESS=""
while [ $# -gt 0 ]; do
    case "$1" in
        -s) ADB="$ADB -s $2"; shift 2 ;;
        --edge) EDGE=$2; shift 2 ;;
        --address) ADDRESS="--address $2"; shift 2 ;;
        *) echo "unknown argument $1" >&2; exit 2 ;;
    esac
done

# shellcheck disable=SC2086
COMPUTER_CODE=$($DESKLINK pairing-code $ADDRESS)
RESULT=$($ADB shell content call --uri "content://$PACKAGE.shell" --method pair --arg "'$COMPUTER_CODE'")
case "$RESULT" in
    *paired*) echo "The phone now trusts this computer." ;;
    *) echo "The phone refused the pairing: $RESULT" >&2; exit 1 ;;
esac
PHONE_CODE=$($ADB shell content call --uri "content://$PACKAGE.shell" --method pairing-code \
    | sed -n 's/.*result=\(desklink:\/\/pair\/[A-Za-z0-9_-]*\).*/\1/p')
if [ -z "$PHONE_CODE" ]; then
    echo "Could not read the phone's pairing code." >&2
    exit 1
fi
$DESKLINK pair "$PHONE_CODE" --edge "$EDGE"
echo "Restart sharing on both devices to connect."
