#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."
desklink_target="${DESKLINK_TARGET:-x86_64-unknown-linux-musl}"
cargo build --release --locked --target "$desklink_target"
mkdir -p dist
desklink_stage="$(mktemp -d)"
trap 'rm -rf "$desklink_stage"' EXIT
install -m 755 "target/$desklink_target/release/desklink" "$desklink_stage/desklink"
cp README.md LICENSE CHANGELOG.md CONTRIBUTING.md SECURITY.md "$desklink_stage/"
cp -R docs examples "$desklink_stage/"
tar -czf "dist/desklink-$desklink_target.tar.gz" -C "$desklink_stage" desklink README.md LICENSE CHANGELOG.md CONTRIBUTING.md SECURITY.md docs examples
printf 'Created dist/desklink-%s.tar.gz\n' "$desklink_target"
