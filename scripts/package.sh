#!/bin/sh
# Package an already built native executable. No upload or Git mutation.
set -eu

binary=${3:-target/release/repot}
actual_version=$("$binary" --version)
actual_version=${actual_version#repot }
version=${1:-$actual_version}
target=${2:-$(rustc -vV | sed -n 's/^host: //p')}
destination=${4:-dist}

case "$version" in
  ''|*[!0-9.]*) echo "release version must be MAJOR.MINOR.PATCH" >&2; exit 1 ;;
esac
if ! printf '%s\n' "$version" | LC_ALL=C grep -Eq '^[0-9]+\.[0-9]+\.[0-9]+$'; then
  echo "release version must be MAJOR.MINOR.PATCH" >&2
  exit 1
fi
if [ "$version" != "$actual_version" ]; then
  echo "release version does not match the executable" >&2
  exit 1
fi
case "$target" in
  x86_64-unknown-linux-gnu|aarch64-unknown-linux-gnu|x86_64-apple-darwin|aarch64-apple-darwin) ;;
  *) echo "unsupported release target" >&2; exit 1 ;;
esac
if [ "$target" != "$(rustc -vV | sed -n 's/^host: //p')" ]; then
  echo "release target must match the native build host" >&2
  exit 1
fi
mkdir -p "$destination"
stage=$(mktemp -d)
trap 'rm -rf "$stage"' EXIT HUP INT TERM
cp "$binary" "$stage/repot"
cp LICENSE README.md "$stage/"
archive="repot-$version-$target.tar.gz"
tar -czf "$destination/$archive" -C "$stage" repot LICENSE README.md
(
  cd "$destination"
  if command -v sha256sum >/dev/null 2>&1; then
    sha256sum "$archive" > "$archive.sha256"
  else
    shasum -a 256 "$archive" > "$archive.sha256"
  fi
)
printf '%s\n' "$destination/$archive"
