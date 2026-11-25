#!/usr/bin/env bash
set -euo pipefail
tmp=$(mktemp -d)
trap 'rm -rf -- "$tmp"' EXIT

target=$(node -p "process.platform + '-' + process.arch + (process.platform === 'linux' ? (process.report.getReport().header.glibcVersionRuntime ? '-gnu' : '-musl') : process.platform === 'win32' ? '-msvc' : '')")
binary=${1:-target/release/wk}
node scripts/npm-package.mjs check
node scripts/npm-package.mjs stage "$target" "$binary" "$tmp/platform"
platform=$(npm pack "$tmp/platform" --silent --pack-destination "$tmp")
launcher=$(npm pack --silent --pack-destination "$tmp")

bash scripts/test-npm-artifacts.sh "$tmp/$launcher" "$tmp/$platform"
