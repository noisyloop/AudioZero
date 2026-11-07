# AudioZero security review

**Scope:** `main` at `3f17796` (arrangement DAW, Operator, VST3 hosting, CI). Reviewed: project load/save (`src/model/ProjectIO.cpp`), sample decoding (`src/model/Project.cpp`), plugin hosting (`src/plugins/PluginManager.cpp`), audio engine (`src/engine/AudioEngine.cpp`), import/drag-and-drop (`src/ui/Session.cpp`, `src/ui/Browser.cpp`), startup (`src/Main.cpp`, `src/ui/MainComponent.cpp`), build and CI (`CMakeLists.txt`, `.github/workflows/`).

The behaviours listed below were checked against the JUCE 8.0.10 sources (`CharacterFunctions::readDoubleValue`, `jlimit`, `WavAudioFormatReader`). None of the findings were run against a live build.

## Threat model

AudioZero is a desktop app, so the attack surface is **files a user opens**:

- `.azproj` projects, which AudioZero is registered to open (`DOCUMENT_EXTENSIONS azproj`), so a double-click in Downloads or an email client is enough;
- audio and MIDI files that are dragged in, imported, or previewed from the browser;
- `.vst3` bundles.

Collaborators routinely pass project files around ("here's my session, add vocals"), so "the user opened a project file from someone else" is the scenario that matters most. Today, opening a project gives its author roughly the same power as running a program they wrote.

Third-party plugins are trusted code by design, because every DAW runs them. The aim is to load them only when the user expects it, never because a project file asked for an arbitrary path.

## Findings

| # | Severity | Finding |
| --- | --- | --- |
| 1 | **Critical** | A project file can make AudioZero load and execute any binary path as a VST3 |
| 2 | High | Re-opening the last project on launch turns finding 1 into persistence and makes crash loops possible |
| 3 | High | A tiny audio file with a forged header causes a multi-GB allocation or an `int` overflow |
| 4 | High | Unvalidated `offset` lets a project cause an out-of-bounds read on the audio thread (arm64) |
| 5 | Medium | `nan` in numeric attributes passes `jlimit` and poisons the engine; inf/NaN reach the audio device |
| 6 | Medium | Unbounded gain, length and count values: full-scale output, an endless WAV export, memory exhaustion |
| 7 | Medium | Plugin state blobs from the project go straight into third-party parsers in-process |
| 8 | Medium | Plugin scanning and loading run in-process on the message thread |
| 9 | Low | Sample paths in projects are absolute and unrestricted (local file probing; UNC paths on Windows) |
| 10 | Low | Supply chain: JUCE pinned by moveable tag; actions pinned by tag; no `permissions:` in workflows |
| 11 | Low | Unsigned builds and no integrity data for release artefacts |
| 12 | Info | Smaller robustness notes |

### 1. A project file can execute any binary as a plugin (Critical)

`io::readDevice` (`src/model/ProjectIO.cpp`) reads a `<PLUGIN>` element straight into a `juce::PluginDescription` and calls `PluginManager::createPlugin`. JUCE's VST3 host then loads `description.fileOrIdentifier` with `dlopen` / `LoadLibrary` / `CFBundleLoad` and runs its static initialisers and factory. The path is never checked against `knownPlugins`, the scanned folders, or the user's intent. `PluginManager::findByIdentifier` exists, but only the browser uses it.

**Scenario:** someone sends `song.zip` containing `song.azproj` and `song Samples/fx.vst3` (renamed malware), with a `<PLUGIN file="...">` pointing at it. On Windows the path can also be a UNC path (`\\host\share\x.vst3`), so no extracted file is needed. The victim double-clicks the project and the code runs with their privileges. This happens before any UI appears, and the attacker also gets finding 2.

**Fix (next phase, first item):**

- When loading a project, resolve each plugin by **identity** (`createIdentifierString()`, or `uniqueId`/`deprecatedUid` plus name and manufacturer) against `knownPlugins`. Never use the file path from the project.
- Use only plugins that were found by a scan of the default VST3 folders or folders the user added, or that the user dragged in themselves.
- If the project needs a plugin that isn't known, show it as missing (a placeholder device that keeps its state blob so saving doesn't lose it) and offer "Locate…", which runs the scan path on a folder the user picks.
- Optional: store `fileOrIdentifier` only as a hint, and show it in the missing-plugin dialog.

### 2. Auto-reopening the last project (High)

`MainComponent` reopens `settings().getValue("lastProject")` on every launch. After finding 1 runs once, it runs again on every launch. Without any malice, a project that crashes on load (bad plugin, or finding 3 or 4) also makes the app unlaunchable until the user deletes the settings file by hand.

**Fix:** write a "loading" marker before the auto-open and clear it after a successful install. If the marker is still there at the next launch, skip the auto-open and offer "Open last project / Open in safe mode (no plugins)". A **safe mode** that loads every plugin as a placeholder is also useful for support.

### 3. Forged audio headers cause huge allocations / `int` overflow (High)

`AudioSample::load` (`src/model/Project.cpp`) limits length with `reader->sampleRate * 60 * 20`. Both values come from the file header:

- a WAV with `sampleRate = 4,000,000` and a `data` chunk that **claims** ~4 GB (JUCE takes `lengthInSamples` from the chunk header, not from the file size) passes the 20-minute check;
- `static_cast<int>(reader->lengthInSamples)` then truncates or turns negative, and `juce::AudioBuffer<float>(channels, n)` tries to allocate up to ~17 GB or gets a negative size;
- the result is `std::bad_alloc` (uncaught, so the app terminates), OOM, or undefined behaviour.

The file can be a few hundred bytes. It triggers on project load, drag-and-drop, and a single click in the browser (preview).

**Fix:**

- Require `8000 <= sampleRate <= 384000`.
- Cap total frames as an absolute number, for example `<= 384000 * 60 * 20`, and check it fits in `int`.
- Check the claimed size is plausible against `file.getSize()` for uncompressed formats.
- Catch `std::bad_alloc` around decoding.
- Decode on a background thread so a slow or hostile file can't freeze the UI.

### 4. Out-of-bounds read from `offset` on the audio thread (High on arm64)

`renderAudioClips` computes `srcPos = (tInClip + clip.sampleOffset) * sample.sampleRate` and then `idx = static_cast<int>(srcPos)`. `offset` comes from the project unvalidated. For `offset="1e12"`, converting an out-of-range double to `int` is undefined behaviour:

- on arm64 (all Apple Silicon Macs) `fcvtzs` saturates to `INT_MAX`;
- then `idx + 1` overflows to `INT_MIN`, which passes `idx + 1 >= srcFrames`;
- so `src[INT_MAX]` is read on the real-time thread.

The result is a crash, or reading process memory into the audio output. On x86 the conversion gives `INT_MIN` and the sample is skipped, but it's still undefined behaviour.

**Fix:**

- Clamp `offset` to `[0, sample length in seconds]` when loading.
- In the engine, compare `srcPos` as a `double` against `srcFrames - 1` **before** converting to an integer.

The same pattern also appears in `renderPreview`, `toOffset` in `addClipMidi`, `first`/`last` in `renderAudioClips`, and `exportWav`'s `totalSamples`.

### 5. NaN passes validation (Medium)

JUCE's `getDoubleAttribute` parses `"nan"` and `"inf"`. `jlimit(20.0, 999.0, NaN)` returns NaN because both comparisons are false. So `bpm="nan"` gets through:

- `samplesPerBeat` and the playhead become NaN;
- every later cast to `int` is undefined behaviour;
- plugins receive a NaN tempo through `getPosition()`, and many crash on it.

`masterVolume`, `volume`, `pan`, `gain`, `loopStart`/`loopEnd`, and note start/length/velocity have no validation at all. The safety clipper (`a > 0.9f`) is false for NaN, so **NaN goes to the audio driver**. On some interfaces that causes full-scale pops.

**Fix:**

- Add one `readFinite(e, name, def, lo, hi)` helper that rejects non-finite values and clamps, and use it for every numeric attribute in `io::load`.
- In the engine, replace non-finite samples with 0 before the clipper (or as part of it).

### 6. Unbounded magnitudes (Medium)

- **Volume and gain:** `volume="1e6"` or a huge `gain`/`masterVolume` drives every track into the clipper at full scale. That's a hearing and speaker risk, especially on headphones. Clamp to the UI ranges (for example volume 0–2, gain 0–4, pan −1–1).
- **Lengths:** with `length="1e300"` on a clip, `songEndBeats()` makes `exportWav` loop for practically forever, synchronously on the message thread, while writing an ever-growing WAV. Clamp `start`/`length` (for example ≤ 100,000 beats) and cap export length.
- **Counts:** the numbers of tracks, clips, notes and effects are unbounded. A few-MB XML file with millions of `<Note>`s, or thousands of tracks each with an instrument, can exhaust memory or stall the audio thread, which scans every note on every block. Set limits and fail the load with a clear message.
- **File size:** `XmlDocument::parse(file)` reads the whole file. Reject project files over a sane size (for example 256 MB) before parsing.

### 7. Plugin state from the project goes into third-party parsers (Medium)

`setStateInformation` receives an attacker-chosen base64 blob, both for built-ins and for third-party plugins. The built-ins only accept XML with the right tag, and APVTS clamps parameter values, so they are fine. Third-party plugins have historically had weak state parsers, so a malicious project could target a popular plugin the victim has installed.

You can't fix plugin code, but you can:

- cap the blob size (for example 64 MB);
- load state only after the plugin was resolved through finding 1's identity check;
- move plugins out of process (finding 8), so a parser bug can't take over AudioZero.

### 8. In-process scanning and hosting (Medium)

The README already calls this out. `PluginDirectoryScanner` and every instantiation run in AudioZero's process on the message thread. A hanging plugin freezes the UI, a crashing one kills the session, and a malicious one has full access. The dead-man's-pedal file helps with crashes during scans only.

**Next phase:**

- Do scans out of process first. JUCE supports a custom `CustomScanner` that spawns a child process, and this is cheap.
- Later, add a plugin bridge process (one per plugin or per vendor) for hosting, with a watchdog.

### 9. Unrestricted sample paths (Low)

`file` / `relativeFile` can name any path:

- an absolute path anywhere on disk;
- `../../` outside the project folder;
- UNC/SMB paths on Windows. Opening one leaks the user's NTLM hash to the remote host, a known Windows credential-leak primitive.

The impact is limited because content is only decoded locally, but it does let a project probe for the existence of files.

**Fix:**

- Prefer `relativeFile` and keep it inside the project folder or the user's sample folders.
- Reject UNC and `\\?\` paths.
- Treat the absolute path only as a fallback the user confirms ("This project references files outside its folder").

### 10. Supply chain and CI (Low)

- **JUCE pin:** `FetchContent_Declare(juce ... GIT_TAG 8.0.10)` pins by tag. Tags can be moved, and someone who takes over the upstream repo or the network path could ship different code. Pin to the commit SHA (keep the tag in a comment) and consider vendoring or mirroring JUCE for release builds.
- **Action pins:** `actions/checkout@v4` and `actions/upload-artifact@v4` are tag pins. Pin to full SHAs and let Dependabot (`.github/dependabot.yml`, `package-ecosystem: github-actions`) bump them.
- **Token permissions:** neither workflow sets `permissions:`, so the token gets the repository default, which may be write. Add `permissions: contents: read` at the top level. Pull requests from forks already get a read-only token, and there's no `pull_request_target`, which is good.
- **Artefact retention:** CI artefacts are downloadable unsigned binaries. Set `retention-days`, and don't present them as releases.

### 11. Unsigned builds and no artefact integrity (Low)

The README already lists signing as planned.

- **macOS:** Developer ID signing, the hardened runtime, and notarization. Under the hardened runtime, loading third-party plugins needs the `com.apple.security.cs.disable-library-validation` entitlement. Keep the entitlements otherwise minimal: no `allow-unsigned-executable-memory` or `allow-dyld-environment-variables` unless a plugin really needs them.
- **Windows:** Authenticode signing.
- **All platforms:** publish SHA-256 checksums with releases. Consider SLSA provenance or `actions/attest-build-provenance` once releases exist.

### 12. Info / robustness

- `AudioSample::load`'s static cache isn't thread-safe and never evicts dead entries. It's fine while everything runs on the message thread, but it will break as soon as decoding moves to a background thread (finding 3). Protect it with a mutex.
- The `--screenshot <path>` option deletes and overwrites the path it's given. That's fine as a developer flag; consider compiling it only into non-release builds.
- With no saved folders, the browser scans the user's Desktop or Music folder at startup. On macOS that triggers a privacy (TCC) prompt on first run. Consider asking first.
- Exceptions thrown out of plugin calls (`processBlock`, `setStateInformation`, `createPluginInstance`) aren't caught. A `try`/`catch` around load-time calls at least turns some failures into a "plugin failed to load" message.

## Suggested next phase: "Hardening I"

These are in priority order and could each be a small PR.

1. **Trust plugins by identity, not by path** (finding 1).
   - Resolve devices through `knownPlugins`.
   - Use placeholder devices that keep their state for missing plugins.
   - Add a "Locate plugin…" flow.
   - Test: a project that names an unknown `.vst3` path loads with a missing-plugin warning and never calls `createPluginInstance`.
2. **Validate the project schema** (findings 5, 6, 9).
   - Use a `readFinite` helper for every numeric attribute, plus limits on counts and lengths, plus a cap on file size.
   - Add a `formatVersion` check: refuse newer versions with a clear message.
   - Resolve sample paths relative to the project first and reject UNC paths.
3. **Harden audio decoding** (findings 3, 4).
   - Bound sample rate and frame count, catch `bad_alloc`, decode in the background.
   - Do double-domain bounds checks in `renderAudioClips` / `renderPreview`.
   - Add a NaN/inf scrub before the master clipper.
4. **Recover from crashes on launch** (finding 2).
   - A load marker, no auto-open after a crash, and a safe mode that loads no plugins.
   - This fits with the autosave / crash-recovery item already on the roadmap.
5. **Fuzzing in CI.** The core sources already build headless for `AudioZeroCoreTests`.
   - Add a libFuzzer target (`-fsanitize=fuzzer,address,undefined`, Linux/clang) for `io::load` (with a stub `PluginManager` that records plugin requests and never loads them), `AudioSample::load`, and `importMidiFile`.
   - Seed it with the demo project and a few WAV/MIDI files.
   - Run it for a few minutes on each PR, and longer nightly.
   - Run the existing tests under ASan/UBSan on Linux as a separate CI job. That alone would flag findings 4 and 5.
6. **CI and supply chain** (finding 10).
   - `permissions: contents: read`, SHA-pinned actions plus Dependabot, JUCE pinned to its commit SHA.
7. **Out-of-process plugin scanning** (finding 8). This is the first step toward the sandboxing already on the roadmap.
8. **Signing and notarization** (finding 11), with a minimal entitlements file and published checksums.

### Before the MCP server ships

The roadmap's MCP server will expose `Session` operations to an AI client. That makes prompts and anything the model reads (web pages, other files) into a new way to drive AudioZero.

- **Transport:** use stdio, or bind to `127.0.0.1` only with a per-launch random token. Never listen on `0.0.0.0` without auth. Check the `Origin` header to block DNS-rebinding requests from browsers.
- **No file system or plugin primitives:** don't expose generic "load file", "load plugin by path" or "save to path" tools. Expose only:
  - plugins by known identity (finding 1);
  - files inside the project folder or the user's sample folders;
  - saving to the current project, or to new files under `~/Music/AudioZero`.
- **Confirmation:** destructive or expensive operations (export, overwriting saves, deleting tracks, loading third-party plugins) should require confirmation in the UI or an explicit capability setting.
- **Same validation as file load:** every numeric argument goes through the same `readFinite`/limits layer as project loading, so the two can't drift apart. Rate-limit calls so a runaway agent can't stall the message thread.
- **Undo and audit:** every MCP edit goes through the undo stack (already true for arrangement edits; extend it to devices) and is listed in a visible activity log, so the user can see and revert what the assistant did.
