# Windows Port Plan

Goal: ship a Windows version of Videorc that hits the project's real bar — a
smooth preview and a correct recording (docs/, memory: OBS parity is dropped).
Dark-glass UI carries over; macOS-only niceties degrade gracefully.

## Where we stand (audited 2026-06-12)

The codebase is in better shape for this than expected. The platform seams
already exist:

- **Backend ↔ app transport is portable.** Axum WebSocket on `127.0.0.1` with
  token auth, `READY {host, port, token}` handshake over stdout, OAuth
  callback listener on loopback TCP. Nothing to change.
- **Every macOS framework is already `cfg`-gated** (`screen_capture.rs`,
  `camera_capture.rs`, `audio.rs`, `video_toolbox_encoder.rs`,
  `metal_compositor.rs`, …) with non-macOS stubs that return empty lists or
  bail. The backend should be *near*-compilable for Windows today.
- **The capture/encode hot path is ffmpeg subprocesses**, not frameworks.
  ScreenCaptureKit/AVFoundation are used for *discovery* (device lists,
  format matrices); recording assembles ffmpeg arg lists. VideoToolbox and
  the Metal compositor are opt-in sidecars with CPU/ffmpeg fallbacks.
- **The Electron app is ~95% portable.** Shortcuts already check
  `metaKey || ctrlKey`; mac-only code (dock icon, vibrancy, wallpaper fetch,
  System Settings deep links) is behind `process.platform` guards or
  degrades gracefully.

The real Windows work concentrates in five places:

1. **ffmpeg input plumbing** — recording/preview build `-f avfoundation`
   inputs and `screen:screencapturekit:` / `camera:avfoundation-native:`
   device-ID schemes (`recording.rs:4098`, `recording.rs:2826`). Windows
   needs `ddagrab`/`gdigrab` (screen) and `dshow` (camera/mic) equivalents
   plus new ID schemes.
2. **Device discovery** — Windows implementations behind the existing stubs:
   display/window enumeration, camera + format matrix, microphones.
3. **Unix-isms** — `libc::mkfifo` audio/overlay FIFOs (`audio.rs:237`,
   `recording.rs:4185`), Unix signals + `libc::kill` orphan watchdog
   (`main.rs`), macOS paths in `storage.rs`, Keychain in `secrets.rs`.
4. **ffmpeg + packaging** — a Windows ffmpeg (LGPL: **no libx264**; the
   encoder analog of VideoToolbox is MediaFoundation `h264_mf`, plus
   NVENC/QSV/AMF), `win:` section in electron-builder, `.exe` handling.
5. **Window chrome** — `vibrancy`, `titleBarStyle: 'hiddenInset'`, traffic
   lights, and the osascript wallpaper fetch are mac-only; Windows needs its
   own glass expression and window controls.

## Phase 0 — Prerequisites and decisions

Decisions to make before any code; each unblocks a later phase.

- **Hardware.** A real x64 Windows 11 machine with a GPU is strongly
  recommended (capture + hardware-encode behavior can't be judged in a VM;
  preview smoothness is judged by eye per project memory). A Windows ARM VM
  on the Mac (Parallels/UTM) is fine for Phase 1 bring-up only.
- **CI reality.** GitHub Actions budget is exhausted (memory) — plan around
  local gates on the Windows box, mirroring `smoke:local-gates`. Optional
  later: self-hosted runner on that box.
- **ffmpeg sourcing.** Start with a pinned prebuilt **LGPL win64** build
  (e.g. BtbN `win64-lgpl` release) checked into `vendor/ffmpeg/` layout;
  write `scripts/fetch-ffmpeg-windows.ps1` later if we want reproducible
  in-house builds (mingw-w64 cross-compile is possible but is its own
  project). Must-have components: `ddagrab`/`gdigrab`, `dshow`,
  `h264_mf`/`hevc_mf`, `h264_nvenc`, `h264_qsv`, `h264_amf`, mpegts/mp4
  muxers, flv/rtmp for streaming.
- **Minimum Windows version: Windows 11 only (build 22000+) — DECIDED
  2026-06-12.** Buys Mica/acrylic, mature Windows.Graphics.Capture, and
  `IDesktopWallpaper` per-monitor wallpaper for the glass underlay.
  electron-builder can't enforce a floor for win targets, so Phase 1 adds a
  runtime check (`os.release()` build ≥ 22000) with a friendly quit dialog.
- **Crate choice:** `windows` (windows-rs) for all Win32/WinRT/COM work —
  DXGI, MediaFoundation, WASAPI, Job Objects.

## Phase 1 — It builds, launches, and connects (no capture)

Outcome: app starts on Windows, glass-ish UI renders, backend spawns and
connects over loopback, device lists are empty but nothing crashes.

Backend:
- Add Windows clauses to `storage.rs` paths (`%APPDATA%\Videorc`,
  `%USERPROFILE%\Videos\Videorc\Recordings`) and `secrets.rs`
  (Windows Credential Manager; `keyring`-style abstraction).
- Replace Unix signal handling/orphan watchdog in `main.rs` with
  `tokio::signal::ctrl_c` + a Job Object ("kill on job close") so the
  backend and its ffmpeg children die with the app — this is *better* than
  the PID-ledger semantics and worth doing first, not as polish.
- Gate the FIFO helpers (`audio.rs`, `recording.rs`) behind
  `cfg(unix)` so the crate compiles; Windows replacements come in Phase 2/3.
- Gate: `cargo check --target x86_64-pc-windows-msvc` (cross-check runs on
  the Mac — catches type errors without the Windows box).

Electron:
- `.exe` suffixes in backend/ffmpeg resolution (`main/index.ts:2313-2554`),
  spawn semantics, owned-process cleanup via the Job Object.
- OAuth on Windows: `open-url` does not fire; the `videorc://` URL arrives
  in `process.argv` of the second instance — handle it in the existing
  `second-instance` listener.
- Window chrome v1: `titleBarStyle: 'hidden'` + `titleBarOverlay` (native
  min/max/close, themed to the glass tokens); skip transparency initially —
  solid `--background` fallback is already the degraded glass path.
- electron-builder: `win:` section (`nsis` + `dir` targets), `icon.ico`,
  protocol registration; unsigned builds for now. Runtime Windows 11 guard
  (build ≥ 22000) with a quit dialog, since the installer can't enforce it.
- Gate: `pnpm package` on Windows produces a launchable app;
  `smoke:dev`-class scripts (the portable ones) pass.

## Phase 2 — Recording MVP via ffmpeg (the tracer bullet)

Outcome: pick a screen + camera + mic on Windows, see previews, record a
correct file, push a stream. This is the slice that proves the product on
Windows.

**Architecture insight (from the grill):** in the default encoder-bridge
path, devices are owned by the *preview* pipelines; recording composites
from their frame stores (`recording.rs:532-615` → `compositor.rs:314-316`)
and opens no device of its own. So the Windows capture work lands in
`preview_camera.rs` / `preview_screen.rs` first, and recording follows
almost for free. Build one shared per-platform input-builder (device ID →
ffmpeg input args) used by previews *and* the legacy direct-capture path
(fps > 30), so both routes get Windows support from the same seam —
extracting that seam is a mac-side refactor slice that existing smokes can
verify before any Windows code lands.

- **Screen:** enumeration via DXGI outputs (`windows` crate) behind the
  `screen_capture.rs` stub; ID scheme `screen:dxgi:<adapterLuid>:<output>`;
  input via `-f lavfi ddagrab=...,hwdownload,format=bgra` (GPU Desktop
  Duplication; the bridge consumes raw frames over a pipe, hence the
  download) with `gdigrab` fallback. Display capture only — window capture
  is *also* display-fallback on macOS today (`recording.rs:5268`
  "window-capture-fallback"), so Windows v1 owes nothing there.
- **Camera:** enumeration via MediaFoundation `MFEnumDeviceSources` +
  format matrix behind `camera_capture.rs` stub; ID = MF symbolic link
  (stable across renames/duplicates; dshow accepts it as
  `video=@device_pnp_…`); capture via `-f dshow`.
- **Mic:** `-f dshow` audio input + `-af volume=<gain>dB` (mute → drop the
  input). Verified: gain/mute are start-time params with no live-update
  command (`protocol.rs:592-594`, no Set/Update handler), so the filter
  gives FULL functional parity with the mac native path — no feature loss
  in the MVP. The native WASAPI port is Phase 3 and motivated by epoch
  alignment + future system audio, not by missing knobs.
- **Encoder:** default `h264_mf` (MediaFoundation = the VideoToolbox analog
  in LGPL ffmpeg); startup one-frame probe (pattern exists:
  `VIDEORC_ENCODER_BRIDGE_VIDEOTOOLBOX_PROBE`) preferring
  NVENC → QSV → AMF → MF. Wire the same
  `VIDEORC_ENCODER_BRIDGE_VIDEO_OUTPUT` switchboard.
- **Streaming rides along:** the RTMP chain is already portable — flv/tee
  muxers with per-leg fifo isolation (`recording.rs:3872-3892`); the only
  mac-ism is the literal `h264_videotoolbox` codec name
  (`recording.rs:3816`), which the encoder probe replaces. Include one
  multistream smoke in this phase's gate rather than deferring.
- Preview stays on the existing portable frame-polling surface (the
  IOSurface/CAMetalLayer zero-copy driver is mac-only and explicitly
  optional).
- Gate: `analyze-recording`/`check-real-source-evidence` pass on a Windows
  recording; A/V sync baseline within the same thresholds as macOS; one
  `smoke:multistream`-class run against a test RTMP sink; preview judged by
  eye on a moving scene (freezedetect per memory).

## Phase 3 — Native parity where it earns its keep

Outcome: Windows quality matches macOS daily-driver quality.

- **WASAPI mic capture** ported into `audio.rs`'s ring-buffer design, with
  the FIFO replaced by a named pipe (`\\.\pipe\videorc-audio-…`) or stdin
  pipe; restores mono→stereo handling and video-epoch alignment (gain/mute
  already have parity via the Phase 2 volume filter). Design the module
  WASAPI-loopback-ready: system audio is plan-only on macOS today
  (docs/system-audio-capture-plan.md, `DeviceKind::SystemAudio` always
  Unavailable), and when SA lands, Windows loopback capture is the *easy*
  platform — don't paint it out.
- **Windows.Graphics.Capture** only if/when macOS grows native window
  capture — today window selection falls back to display recording on BOTH
  platforms, so this is parity-neutral. Stay on ddagrab unless Phase 2
  measurements say otherwise.
- **Encoder bridge sidecar:** add a `WindowsMediaFoundationH264` variant to
  `EncoderBridgeVideoOutput` mirroring the VideoToolbox sidecar, if ffmpeg
  `h264_mf` proves limiting (measure first — it may not).
- **Overlay FIFO** (`recording.rs:4185`) → named pipe, only if the overlay
  feature is in the Windows scope.
- Explicit non-goals: Metal compositor port (CPU YUV path is the live path
  today), IOSurface zero-copy preview, native preview host windows (AppKit
  helper binary stays mac-only).

## Phase 4 — Windows-native glass and UX

Outcome: the design language reads as intentional on Windows, not as a mac
app in exile.

- **Glass:** two candidates, A/B by eye on the Windows box:
  (a) Electron `backgroundMaterial: 'acrylic'|'mica'` (Win 11) — real system
  blur, but constrains frame options; (b) port the `GlassWallpaperUnderlay` —
  wallpaper path on Windows is a registry read (`Control Panel\Desktop\WallPaper`),
  no permission prompt needed, and the renderer underlay already does the
  blur. (b) preserves the existing architecture and identical look; likely
  winner.
- **Kbd glyphs:** expose platform via preload; `⌘` → `Ctrl` in `kbd.tsx`
  and the footer/palette hints.
- **Permissions UX:** `ms-settings:privacy-webcam` / `privacy-microphone`
  deep links replacing `x-apple.systempreferences:`; note Windows has no
  screen-recording permission gate.
- Theme toggle, focus rings, reduced-motion: re-verify with
  `ui-theme-screens.mjs` ported (its screenshots go through CDP, not
  `screencapture` — should port nearly for free).

## Phase 5 — Packaging, signing, verification harness

- NSIS installer + portable dir; ffmpeg + backend.exe in `resources`;
  LGPL compliance is already satisfied by shipping ffmpeg as a separate
  spawned binary (same as macOS).
- Code signing: Azure Trusted Signing (cheapest route past SmartScreen) or
  an OV/EV Authenticode cert; unsigned is fine for internal testing only.
- Port the smoke harness tier by tier: the ~30 portable smokes first
  (`smoke:dev`, `smoke:oauth*`, `smoke:sources`, lifecycle, multistream),
  then baselines (`real-source-baseline-app.mjs` needs the Windows device
  IDs from Phase 2), leaving the ~12 `screencapture`-based `ui-*` probes
  mac-only or on CDP screenshots.
- Define `smoke:local-gates:windows` and run it on the Windows box as the
  merge gate (no Actions budget).

## Risks / open questions

- **Hardware encoder variance** (NVENC vs QSV vs AMF vs MF-software) is the
  biggest quality unknown; budget probe time on at least one discrete-GPU
  and one iGPU machine.
- **Electron transparency on Windows** is historically buggy
  (maximize/snap glitches) — hence chrome v1 ships solid, glass lands in
  Phase 4 behind the same env-var switches used for the mac glass bisect.
- **dshow camera format negotiation** is messier than AVFoundation's format
  matrix; expect device-specific quirks.
- **ddagrab + multi-GPU laptops** (Optimus): adapter selection needs an
  explicit flag and a fallback.
- **Dev loop friction:** one developer on macOS, app behavior on Windows —
  budget for remote-access tooling to the Windows box (SSH + screenshots or
  Parsec) so the judge-by-eye loop stays tight.

## Sequencing note

Phases 0–2 are the critical path and deliberately lean on ffmpeg for
everything; that's the shortest route to "a correct recording on Windows."
Phases 3–5 are quality/productization and can interleave with ongoing macOS
work. Two slices run entirely on the Mac and should come first:
1. `cargo check --target x86_64-pc-windows-msvc` green — forces every
   Unix-ism into the open. **DONE 2026-06-12:** `pnpm check:windows`
   (cargo-xwin + Homebrew LLVM for `llvm-lib`; the Unix-isms were the five
   FIFO helpers — now one `fifo.rs` seam with a Windows stub — and ungated
   `metal_compositor::source_zerocopy_enabled` calls in the two preview
   modules).
2. Extract the per-platform ffmpeg input-builder seam from
   `recording.rs`/`preview_*.rs` with behavior pinned by the existing
   smokes — so the Windows branch lands in a prepared socket instead of a
   4,000-line file. **DONE 2026-06-12:** new `capture_input.rs` owns the
   `VideoInput`/`MicrophoneInput` enums and the avfoundation/native-FIFO
   input builders (session + live-render arms); `recording.rs` and
   `live_render.rs` call through it. Behavior-neutral — 577 backend tests
   unchanged, Windows cross-check still green. The Windows ddagrab/dshow
   arms slot into this module in Phase 2.

## Grill resolutions (auto-grill, 2026-06-12)

Questions stress-tested against the codebase; answers verified, not
assumed.

| # | Question | Resolution (evidence) |
|---|----------|----------------------|
| 1 | Is streaming in Windows v1 or recording-only? | **In v1, nearly free.** RTMP chain is flv/tee + fifo isolation, fully portable (`recording.rs:3872-3892`); only mac-ism is the `h264_videotoolbox` codec literal (`recording.rs:3816`) which the encoder probe replaces. |
| 2 | Does the dshow-direct mic MVP lose user-facing features vs the native CoreAudio path? | **No.** Native path's gain/mute are start-time params (`protocol.rs:592-594`) with no live-update command; an `-af volume` filter at spawn matches them. The avfoundation *fallback* on mac loses gain/mute today — the Windows MVP with the filter is actually closer to parity than mac's own fallback. |
| 3 | Will preview + recording fight over the same device (dshow opens are exclusive)? | **No contention in the default path.** Encoder-bridge recording composites from the preview pipelines' frame stores (`recording.rs:532-615`, `compositor.rs:314-316`) — one open per device. Implication: port the *preview* capture first; recording follows. The legacy direct path (fps > 30) is the only second-open risk and shares the same input-builder seam. |
| 4 | Must Windows v1 capture individual windows? | **No.** Window selection is metadata-only on macOS too — recording warns `window-capture-fallback` and records the display (`recording.rs:5268-5271`). Display-only is parity. |
| 5 | System audio in scope? | **No — plan-only on macOS** (docs/system-audio-capture-plan.md; `DeviceKind::SystemAudio` always Unavailable). Phase 3's WASAPI module just keeps loopback reachable for when SA lands. |
| 6 | Do device IDs need to be durable across sessions? | **Soft requirement.** Selections travel in per-session params; no device IDs in the sqlite schema (`storage.rs` tables). Still prefer MF symbolic links / adapter LUIDs so remembered selections survive replugs. |
| 7 | Can ddagrab feed the raw-frame pipe the bridge expects? | **Yes, with `hwdownload,format=bgra`** — ddagrab produces D3D11 frames; the bridge consumes raw frames over a pipe. CPU download cost is the thing to measure on the Windows box; `gdigrab` is the fallback. |
| 8 | Windows 10 or 11 floor? | **Windows 11 only — owner decision 2026-06-12.** Runtime guard in Phase 1; unlocks Mica/acrylic and `IDesktopWallpaper`. |

Open items that stay with the owner: which Windows box/GPUs to buy (one
discrete + one iGPU machine ideal), and when to pay for code signing
(Azure Trusted Signing vs unsigned internal builds).
