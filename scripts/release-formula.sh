#!/bin/sh
# Generate a versioned binary formula only from a complete set of actual assets.
set -eu
version=${1:?usage: release-formula.sh VERSION ASSET_DIRECTORY}
assets=${2:?usage: release-formula.sh VERSION ASSET_DIRECTORY}
case "$version" in
  ''|*[!0-9.]*) echo "release version must be MAJOR.MINOR.PATCH" >&2; exit 1 ;;
esac
if ! printf '%s\n' "$version" | LC_ALL=C grep -Eq '^[0-9]+\.[0-9]+\.[0-9]+$'; then
  echo "release version must be MAJOR.MINOR.PATCH" >&2
  exit 1
fi
checksum() {
  file="$assets/repot-$version-$1.tar.gz"
  if [ ! -f "$file" ]; then echo "missing release asset for $1" >&2; exit 1; fi
  if command -v sha256sum >/dev/null 2>&1; then
    sum=$(sha256sum "$file")
  else
    sum=$(shasum -a 256 "$file")
  fi
  printf '%s\n' "${sum%% *}"
}
darwin_arm=$(checksum aarch64-apple-darwin)
darwin_intel=$(checksum x86_64-apple-darwin)
linux_arm=$(checksum aarch64-unknown-linux-gnu)
linux_intel=$(checksum x86_64-unknown-linux-gnu)
cat > "$assets/repot.rb" <<EOF
class Repot < Formula
  desc "Keep Git repositories organised, current and portable"
  homepage "https://github.com/niklas-heer/repot"
  head "https://github.com/niklas-heer/repot.git", branch: "main"
  version "$version"
  license "MIT"

  on_macos do
    on_arm do
      url "https://github.com/niklas-heer/repot/releases/download/v$version/repot-$version-aarch64-apple-darwin.tar.gz"
      sha256 "$darwin_arm"
    end
    on_intel do
      url "https://github.com/niklas-heer/repot/releases/download/v$version/repot-$version-x86_64-apple-darwin.tar.gz"
      sha256 "$darwin_intel"
    end
  end
  on_linux do
    on_arm do
      url "https://github.com/niklas-heer/repot/releases/download/v$version/repot-$version-aarch64-unknown-linux-gnu.tar.gz"
      sha256 "$linux_arm"
    end
    on_intel do
      url "https://github.com/niklas-heer/repot/releases/download/v$version/repot-$version-x86_64-unknown-linux-gnu.tar.gz"
      sha256 "$linux_intel"
    end
  end

  depends_on "git"
  depends_on "rust" => :build if build.head?

  def install
    if build.head?
      system "cargo", "install", *std_cargo_args
    else
      bin.install "repot"
    end
  end

  test do
    expected = build.head? ? "repot " : "repot #{version}"
    assert_match expected, shell_output("#{bin}/repot --version")
    ENV["HOME"] = testpath
    ENV["GHQ_ROOT"] = testpath/"repos"
    ENV["XDG_CONFIG_HOME"] = testpath/"config"
    mkdir "repos"
    assert_equal "[]", shell_output("#{bin}/repot list --json").strip
  end
end
EOF
(
  cd "$assets"
  if command -v sha256sum >/dev/null 2>&1; then
    sha256sum repot-*.tar.gz repot.rb > SHA256SUMS
  else
    shasum -a 256 repot-*.tar.gz repot.rb > SHA256SUMS
  fi
)
