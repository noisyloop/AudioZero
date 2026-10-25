#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."
cmake -S . -B build -G Xcode -DCMAKE_BUILD_TYPE=Release -DAUDIOZERO_BUILD_TESTS=ON
cmake --build build --config Release --parallel 4
ctest --test-dir build -C Release --output-on-failure
echo "Build complete: build/AudioZero_artefacts/Release/AudioZero.app and build/AudioZeroOperator_artefacts/Release/VST3/"
