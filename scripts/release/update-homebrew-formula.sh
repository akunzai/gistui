#!/usr/bin/env bash
set -euo pipefail

if [ -z "${HOMEBREW_BUMP_TOKEN:-}" ]; then
  echo "HOMEBREW_BUMP_TOKEN is not set; skipping Homebrew formula update."
  exit 0
fi

VERSION="${VERSION:?VERSION must be set (e.g. v0.15.1)}"

git clone "https://x-access-token:${HOMEBREW_BUMP_TOKEN}@github.com/akunzai/homebrew-tap.git" homebrew-tap

SHA_MAC_INTEL=$(awk '{print $1}' "artifacts/gistui-${VERSION}-x86_64-apple-darwin.tar.gz.sha256")
SHA_MAC_ARM=$(awk '{print $1}' "artifacts/gistui-${VERSION}-aarch64-apple-darwin.tar.gz.sha256")
SHA_LINUX_INTEL=$(awk '{print $1}' "artifacts/gistui-${VERSION}-x86_64-unknown-linux-gnu.tar.gz.sha256")
SHA_LINUX_ARM=$(awk '{print $1}' "artifacts/gistui-${VERSION}-aarch64-unknown-linux-gnu.tar.gz.sha256")

cat <<EOF > homebrew-tap/Formula/gistui.rb
class Gistui < Formula
  desc "Terminal UI for managing GitHub Gists"
  homepage "https://akunzai.github.io/gistui/"
  license "MIT"

  livecheck do
    url :stable
    strategy :github_latest
  end

  head do
    url "https://github.com/akunzai/gistui.git", branch: "main"
    depends_on "rust" => :build
  end

  depends_on "gh" # gistui shells out to the GitHub CLI at runtime

  on_macos do
    if Hardware::CPU.intel?
      url "https://github.com/akunzai/gistui/releases/download/${VERSION}/gistui-${VERSION}-x86_64-apple-darwin.tar.gz"
      sha256 "${SHA_MAC_INTEL}"
    end
    if Hardware::CPU.arm?
      url "https://github.com/akunzai/gistui/releases/download/${VERSION}/gistui-${VERSION}-aarch64-apple-darwin.tar.gz"
      sha256 "${SHA_MAC_ARM}"
    end
  end

  on_linux do
    if Hardware::CPU.intel?
      url "https://github.com/akunzai/gistui/releases/download/${VERSION}/gistui-${VERSION}-x86_64-unknown-linux-gnu.tar.gz"
      sha256 "${SHA_LINUX_INTEL}"
    end
    if Hardware::CPU.arm? && Hardware::CPU.is_64_bit?
      url "https://github.com/akunzai/gistui/releases/download/${VERSION}/gistui-${VERSION}-aarch64-unknown-linux-gnu.tar.gz"
      sha256 "${SHA_LINUX_ARM}"
    end
  end

  def install
    if build.head?
      system "cargo", "install", *std_cargo_args
    else
      bin.install "gistui"
    end
  end

  test do
    assert_match "gistui", shell_output("#{bin}/gistui --help")
  end
end
EOF

cd homebrew-tap
git config user.name "github-actions[bot]"
git config user.email "github-actions[bot]@users.noreply.github.com"
git add Formula/gistui.rb
git commit -m "chore: bump gistui to ${VERSION}" || exit 0
git push origin main
