#!/bin/sh
# Install and test the generated formula on a disposable GitHub macOS runner.
set -eu
assets=${1:?usage: smoke-homebrew.sh ASSET_DIRECTORY}
test "${GITHUB_ACTIONS:-}" = true || {
  echo "Homebrew release smoke tests require a disposable GitHub Actions runner" >&2
  exit 1
}
tap=repot/release-smoke
brew tap-new --no-git "$tap"
tap_path=$(brew --repository "$tap")
python3 - "$assets" "$tap_path/Formula/repot.rb" <<'PY'
from pathlib import Path
import re
import sys

assets = Path(sys.argv[1]).resolve()
formula = (assets / "repot.rb").read_text()
formula, replaced = re.subn(
    r"https://github\.com/niklas-heer/repot/releases/download/v[0-9]+\.[0-9]+\.[0-9]+/",
    assets.as_uri() + "/",
    formula,
)
if replaced != 4:
    raise SystemExit("expected four native release URLs in the generated formula")
Path(sys.argv[2]).write_text(formula)
PY
brew install --formula "$tap/repot"
brew test "$tap/repot"
