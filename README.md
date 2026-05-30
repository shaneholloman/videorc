# Videogre

Videogre is an AI-native desktop studio for creator recording and livestreaming workflows.

This repository currently contains the technical spike:

- Electron + React/TypeScript desktop shell
- Rust backend process launched by Electron
- Authenticated localhost WebSocket protocol
- SQLite-backed local session library
- Device discovery stubs with FFmpeg-backed macOS device probing
- Source, layout, output, RTMP preset, and health event settings
- FFmpeg-backed capture sessions that can record MKV, stream RTMP, or do both through one shared output pipeline
- Optional MP4 remux after MKV recording

Raw media frames do not move through Electron IPC. Electron receives backend connection details, state updates, device metadata, recording status, and logs.

## Prerequisites

- Node.js 24+
- pnpm 11+
- Rust stable via rustup
- FFmpeg available on `PATH`

The spike uses the system FFmpeg binary only. Distribution and closed-source licensing decisions for bundled FFmpeg builds are intentionally out of scope.

## Development

```sh
pnpm install
pnpm dev
```

The desktop app launches the Rust backend automatically. Recordings default to:

```text
~/Movies/Videogre/Recordings
```

Session metadata is stored in:

```text
~/Library/Application Support/Videogre/videogre.sqlite3
```

## Current Phase

Phase 1 is complete. Phase 2 implements the capture session foundation:

- screen/window, camera, and microphone selection
- one v1 layout: screen/window plus camera corner
- camera corner, size, shape, and margin settings
- local recording, RTMP streaming, or record while streaming
- manual RTMP presets for YouTube, Twitch, X, and Custom
- deterministic health events surfaced in the UI
- local session library with MKV to MP4 remux

## Verification

```sh
pnpm typecheck
pnpm build
cargo fmt --check
cargo test
cargo clippy -- -D warnings
```
