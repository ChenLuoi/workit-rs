#!/usr/bin/env bash
set -euo pipefail
tmp=$(mktemp -d)
trap 'rm -rf -- "$tmp"' EXIT

target=$(node -p "process.platform + '-' + process.arch + (process.platform === 'linux' ? (process.report.getReport().header.glibcVersionRuntime ? '-gnu' : '-musl') : process.platform === 'win32' ? '-msvc' : '')")
version=$(node -p "require('./package.json').version")
binary=${1:-target/release/wk}
node scripts/npm-package.mjs check
node scripts/npm-package.mjs stage "$target" "$binary" "$tmp/platform"
platform=$(npm pack "$tmp/platform" --silent --pack-destination "$tmp")
launcher=$(npm pack --silent --pack-destination "$tmp")

# Both tarballs come from this build. Exercise npm's real bin link and optional dependency lookup.
npm install --offline --ignore-scripts --no-audit --no-fund --prefix "$tmp/client" "$tmp/$platform" "$tmp/$launcher"
wk="$tmp/client/node_modules/.bin/wk"
[[ $("$wk" --version) == "wk $version" ]]
git init -q -b main "$tmp/project"
git -C "$tmp/project" -c user.name=test -c user.email=test@example.org commit -qm initial --allow-empty
"$wk" -w "$tmp/test workspace" init --yes --url "$tmp/project" --skills wk-cli
cmp templates/workspace/README.md "$tmp/test workspace/README.md"
cmp templates/workspace/.agents/skills/wk-cli/SKILL.md "$tmp/test workspace/.agents/skills/wk-cli/SKILL.md"
[[ ! -e "$tmp/test workspace/README.zh-CN.md" ]]
"$wk" -w "$tmp/test workspace" doctor
