#!/usr/bin/env bash
# Regenerates the README screenshots from the real app (Linux, needs xvfb-run).
set -euo pipefail
cd "$(dirname "$0")/.."
APP=build/AudioZero_artefacts/Release/AudioZero
for tab in devices pianoroll mixer; do
  xvfb-run -a -s "-screen 0 1920x1200x24" "$APP" --screenshot "docs/screenshots/$tab.png" --tab "$tab"
done
