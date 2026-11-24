#!/usr/bin/env bash
# Build and install tomboy from this checkout, then (re)start the macOS daemon via its
# LaunchAgent (~/Library/LaunchAgents/io.github.dzhibas.tomboy.plist).
set -euo pipefail

if [[ "$(uname -s)" != "Darwin" ]]; then
  echo "install-macos.sh: this script is for macOS" >&2
  exit 1
fi

cd "$(dirname "$0")/.."

cargo install --path . --locked --force

bin="${CARGO_HOME:-$HOME/.cargo}/bin/tomboy"
"$bin" install-launch-agent

echo "tomboy installed at $bin and daemon restarted (log: ~/Library/Logs/tomboy.log)"
