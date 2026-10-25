# Builds AudioZero on Windows. Needs Visual Studio 2022 (Desktop C++ workload) and CMake.
$ErrorActionPreference = "Stop"
Set-Location (Join-Path $PSScriptRoot "..")
cmake -S . -B build -G "Visual Studio 17 2022" -A x64 -DAUDIOZERO_BUILD_TESTS=ON
cmake --build build --config Release --parallel
ctest --test-dir build -C Release --output-on-failure
Write-Host "Run: build\AudioZero_artefacts\Release\AudioZero.exe"
