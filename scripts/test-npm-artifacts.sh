#!/usr/bin/env bash
set -euo pipefail
tmp=$(mktemp -d)
trap 'rm -rf -- "$tmp"' EXIT
version=$(node -p "require('./package.json').version")

# Install the actual release archives with no registry fallback.
npm install --offline --ignore-scripts --no-audit --no-fund --prefix "$tmp/client" "$1" "$2"
wk="$tmp/client/node_modules/.bin/wk"
[[ $("$wk" --version) == "wk $version" ]]
git init -q -b main "$tmp/project"
git -C "$tmp/project" -c user.name=test -c user.email=test@example.org commit -qm initial --allow-empty
"$wk" -w "$tmp/test workspace" init --yes --url "$tmp/project" --skills wk-cli
cmp templates/workspace/README.md "$tmp/test workspace/README.md"
cmp templates/workspace/.agents/skills/wk-cli/SKILL.md "$tmp/test workspace/.agents/skills/wk-cli/SKILL.md"
[[ ! -e "$tmp/test workspace/README.zh-CN.md" ]]
"$wk" -w "$tmp/test workspace" doctor
