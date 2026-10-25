#!/usr/bin/env bash
set -euo pipefail
if [[ "$(uname -s)" != "Darwin" ]]; then
  echo "This inspection script is for macOS." >&2
  exit 2
fi
echo "AudioZero — installed VST3 bundle inventory (not a runtime compatibility test)"
found=0
for dir in "/Library/Audio/Plug-Ins/VST3" "$HOME/Library/Audio/Plug-Ins/VST3"; do
  [[ -d "$dir" ]] || continue
  while IFS= read -r -d '' bundle; do
    found=$((found + 1))
    echo ""
    echo "Bundle: $bundle"
    executable=$(/usr/libexec/PlistBuddy -c 'Print :CFBundleExecutable' "$bundle/Contents/Info.plist" 2>/dev/null || true)
    if [[ -n "$executable" ]]; then
      binary="$bundle/Contents/MacOS/$executable"
      if [[ -f "$binary" ]]; then
        file "$binary"
        codesign --verify --deep "$bundle" 2>&1 || true
      else
        echo "Executable not found at $binary"
      fi
    else
      echo "Missing or unreadable CFBundleExecutable"
    fi
  done < <(find "$dir" -type d -name '*.vst3' -prune -print0)
done
echo ""
echo "Found $found VST3 bundle(s). Runtime loading, audio and state recall are not tested."
