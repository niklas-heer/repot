#!/bin/sh
# Verify and exercise a native release archive in an isolated home.
set -eu
version=${1:?usage: smoke-archive.sh VERSION TARGET ASSET_DIRECTORY}
target=${2:?usage: smoke-archive.sh VERSION TARGET ASSET_DIRECTORY}
assets=${3:?usage: smoke-archive.sh VERSION TARGET ASSET_DIRECTORY}
archive="repot-$version-$target.tar.gz"
(
  cd "$assets"
  if command -v sha256sum >/dev/null 2>&1; then
    sha256sum -c "$archive.sha256"
  else
    shasum -a 256 -c "$archive.sha256"
  fi
)
temporary=$(mktemp -d)
trap 'rm -rf "$temporary"' EXIT HUP INT TERM
tar -xzf "$assets/$archive" -C "$temporary"
test "$("$temporary/repot" --version)" = "repot $version"
mkdir -p "$temporary/home" "$temporary/projects" "$temporary/config"
printf '[settings]\nowners = ["release-smoke"]\n' > "$temporary/config/repos.toml"
printf 'settings { owners "release-smoke"; }\n' > "$temporary/config/repos.kdl"
(
  cd "$temporary"
  for format in toml kdl; do
    result=$(env HOME="$temporary/home" XDG_CONFIG_HOME="$temporary/config" \
      GHQ_ROOT="$temporary/projects" GIT_CONFIG_NOSYSTEM=1 \
      GIT_CONFIG_GLOBAL="$temporary/gitconfig" \
      "$temporary/repot" --manifest "$temporary/config/repos.$format" list --json)
    test "$result" = '[]'
  done
)
printf 'Verified repot %s archive for %s, including TOML and KDL configuration\n' "$version" "$target"
