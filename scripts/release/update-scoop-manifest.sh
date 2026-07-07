#!/usr/bin/env bash
set -euo pipefail

if [ -z "${HOMEBREW_BUMP_TOKEN:-}" ]; then
  echo "HOMEBREW_BUMP_TOKEN is not set; skipping Scoop manifest update."
  exit 0
fi

VERSION="${VERSION:?VERSION must be set (e.g. v0.15.1)}"

git clone "https://x-access-token:${HOMEBREW_BUMP_TOKEN}@github.com/akunzai/scoop-bucket.git" scoop-bucket

SHA_WIN=$(awk '{print $1}' "artifacts/gistui-${VERSION}-x86_64-pc-windows-msvc.zip.sha256")

jq --indent 4 --arg version "${VERSION#v}" \
   --arg url "https://github.com/akunzai/gistui/releases/download/${VERSION}/gistui-${VERSION}-x86_64-pc-windows-msvc.zip" \
   --arg hash "$SHA_WIN" \
   --arg extract_dir "gistui-${VERSION}-x86_64-pc-windows-msvc" \
   '.version = $version | .architecture["64bit"].url = $url | .architecture["64bit"].hash = $hash | .architecture["64bit"].extract_dir = $extract_dir' \
   scoop-bucket/bucket/gistui.json > /tmp/gistui.json.tmp
mv /tmp/gistui.json.tmp scoop-bucket/bucket/gistui.json

cd scoop-bucket
git config user.name "github-actions[bot]"
git config user.email "github-actions[bot]@users.noreply.github.com"
git add bucket/gistui.json
git commit -m "chore: bump gistui to ${VERSION}" || exit 0
git push origin main
