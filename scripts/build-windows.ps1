# Builds AudioZero on Windows. Needs Visual Studio 2022 or newer (Desktop C++ workload) and CMake.
# CMake picks the newest installed Visual Studio.
$ErrorActionPreference = "Stop"
Set-Location (Join-Path $PSScriptRoot "..")
cmake -S . -B build -A x64 -DAUDIOZERO_BUILD_TESTS=ON
cmake --build build --config Release --parallel
ctest --test-dir build -C Release --output-on-failure
Write-Host "Run: build\AudioZero_artefacts\Release\AudioZero.exe"
