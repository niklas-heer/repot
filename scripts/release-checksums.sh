#!/bin/sh
# Write SHA256SUMS only from a complete set of actual release archives. The
# Homebrew formula lives in niklas-heer/homebrew-tap, which derives it from
# these checksums after publication.
set -eu
version=${1:?usage: release-checksums.sh VERSION ASSET_DIRECTORY}
assets=${2:?usage: release-checksums.sh VERSION ASSET_DIRECTORY}
case "$version" in
  ''|*[!0-9.]*) echo "release version must be MAJOR.MINOR.PATCH" >&2; exit 1 ;;
esac
if ! printf '%s\n' "$version" | LC_ALL=C grep -Eq '^[0-9]+\.[0-9]+\.[0-9]+$'; then
  echo "release version must be MAJOR.MINOR.PATCH" >&2
  exit 1
fi
for target in aarch64-apple-darwin x86_64-apple-darwin aarch64-unknown-linux-gnu x86_64-unknown-linux-gnu; do
  if [ ! -f "$assets/repot-$version-$target.tar.gz" ]; then
    echo "missing release asset for $target" >&2
    exit 1
  fi
done
(
  cd "$assets"
  if command -v sha256sum >/dev/null 2>&1; then
    sha256sum repot-"$version"-*.tar.gz > SHA256SUMS
  else
    shasum -a 256 repot-"$version"-*.tar.gz > SHA256SUMS
  fi
)
