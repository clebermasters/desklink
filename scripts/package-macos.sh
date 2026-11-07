#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."
test "$(uname -s)" = Darwin || { echo 'Build the macOS bundle on a Mac with Xcode command-line tools.' >&2; exit 1; }
cargo build --release --locked --target aarch64-apple-darwin
cargo build --release --locked --target x86_64-apple-darwin
desklink_stage="$(mktemp -d)"
trap 'rm -rf "$desklink_stage"' EXIT
desklink_app="$desklink_stage/Desklink.app"
mkdir -p "$desklink_app/Contents/MacOS" dist
lipo -create target/aarch64-apple-darwin/release/desklink target/x86_64-apple-darwin/release/desklink -output "$desklink_app/Contents/MacOS/desklink"
cp packaging/macos/Info.plist "$desklink_app/Contents/Info.plist"
if test -n "${DESKLINK_SIGN_IDENTITY:-}"; then
  codesign --force --options runtime --timestamp --sign "$DESKLINK_SIGN_IDENTITY" "$desklink_app"
  codesign --verify --strict --verbose=2 "$desklink_app"
else
  codesign --force --sign - "$desklink_app"
  echo 'Development bundle only: ad-hoc signed, not Developer ID signed or notarized.' >&2
fi
ditto -c -k --keepParent "$desklink_app" dist/desklink-macos-universal.zip
if test -n "${DESKLINK_NOTARY_PROFILE:-}"; then
  test -n "${DESKLINK_SIGN_IDENTITY:-}" || { echo 'Notarization requires DESKLINK_SIGN_IDENTITY.' >&2; exit 1; }
  xcrun notarytool submit dist/desklink-macos-universal.zip --keychain-profile "$DESKLINK_NOTARY_PROFILE" --wait
  xcrun stapler staple "$desklink_app"
  ditto -c -k --keepParent "$desklink_app" dist/desklink-macos-universal.zip
fi
echo 'Created dist/desklink-macos-universal.zip'
