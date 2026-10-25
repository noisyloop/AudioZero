#!/usr/bin/env bash
# Builds AudioZero on Linux. Needs: cmake ninja-build libasound2-dev libfreetype-dev libfontconfig1-dev
# libx11-dev libxrandr-dev libxinerama-dev libxcursor-dev libxcomposite-dev libxext-dev libgl1-mesa-dev
set -euo pipefail
cd "$(dirname "$0")/.."
cmake -S . -B build -G Ninja -DCMAKE_BUILD_TYPE=Release -DAUDIOZERO_BUILD_TESTS=ON
cmake --build build --parallel
ctest --test-dir build --output-on-failure
echo "Run: build/AudioZero_artefacts/Release/AudioZero"
