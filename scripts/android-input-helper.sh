#!/usr/bin/env sh
# Starts Desklink's ADB input helper on a connected Android phone (ADB developer mode).
#
# The helper runs as the adb shell user, not root. It accepts commands only from the Desklink
# app, over 127.0.0.1 with a random token delivered through the app's shell-only provider.
# Run again after each phone restart. Stop it from the app ("Stop helper") or with --stop.
#
#   scripts/android-input-helper.sh [-s SERIAL] [--stop]
set -eu
PACKAGE=com.desklink.android
ADB=${ADB:-adb}
if [ "${1:-}" = "-s" ]; then ADB="$ADB -s $2"; shift 2; fi

if [ "${1:-}" = "--stop" ]; then
    $ADB shell "pkill -f com.desklink.android.shell.InputHelper" && echo "Helper stopped." || echo "No helper was running."
    exit 0
fi

APK=$($ADB shell pm path "$PACKAGE" | sed -n 's/^package://p' | head -n 1 | tr -d '\r')
if [ -z "$APK" ]; then
    echo "Desklink is not installed on the phone." >&2
    exit 1
fi
# One helper at a time.
$ADB shell "pkill -f com.desklink.android.shell.InputHelper" >/dev/null 2>&1 || true
$ADB shell "CLASSPATH=$APK nohup app_process /system/bin com.desklink.android.shell.InputHelper \
    > /data/local/tmp/desklink-helper.log 2>&1 &"
sleep 2
if $ADB shell "pgrep -f com.desklink.android.shell.InputHelper" >/dev/null 2>&1; then
    echo "Helper started. In the Desklink app, choose ADB developer mode."
else
    echo "The helper did not start:" >&2
    $ADB shell cat /data/local/tmp/desklink-helper.log >&2
    exit 1
fi
