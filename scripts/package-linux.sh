#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."
desklink_target="${DESKLINK_TARGET:-x86_64-unknown-linux-gnu}"
desklink_features=()
if test "${DESKLINK_HEADLESS:-0}" = 1; then
  desklink_features+=(--no-default-features)
elif [[ "$desklink_target" == *-musl ]]; then
  echo 'The native GUI needs dynamic system graphics libraries. Set DESKLINK_HEADLESS=1 for a static musl CLI.' >&2
  exit 1
fi
cargo build -p desklink --release --locked --target "$desklink_target" "${desklink_features[@]}"
mkdir -p dist
desklink_stage="$(mktemp -d)"
trap 'rm -rf "$desklink_stage"' EXIT
install -m 755 "target/$desklink_target/release/desklink" "$desklink_stage/desklink"
cp README.md LICENSE CHANGELOG.md CONTRIBUTING.md SECURITY.md "$desklink_stage/"
cp -R docs examples packaging/linux "$desklink_stage/"
tar -czf "dist/desklink-$desklink_target.tar.gz" -C "$desklink_stage" desklink README.md LICENSE CHANGELOG.md CONTRIBUTING.md SECURITY.md docs examples linux
printf 'Created dist/desklink-%s.tar.gz\n' "$desklink_target"
