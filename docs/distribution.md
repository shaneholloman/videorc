# Videogre Distribution Notes

Status: packaging foundation plus signed macOS release scaffolding.

## Local Packaging

Build a packaged app directory with the Rust backend included as an extra resource:

```sh
pnpm package:desktop
```

Build the default Electron Builder distribution target:

```sh
pnpm dist:desktop
```

Both commands first run:

```sh
cargo build --release -p videogre-backend
```

The packaged Electron main process launches `videogre-backend` from `process.resourcesPath`, while development still runs the backend through Cargo.

Run the packaged-app recording smoke test after `pnpm package:desktop`:

```sh
pnpm smoke:packaged
```

The smoke script launches the packaged `.app`, waits for the packaged backend to emit `READY`, calls the authenticated backend WebSocket, records a short local MKV test pattern through system FFmpeg, stops the session, and verifies the file exists.

Useful overrides:

```sh
VIDEOGRE_PACKAGED_APP_EXECUTABLE=/path/to/Videogre.app/Contents/MacOS/Videogre pnpm smoke:packaged
VIDEOGRE_SMOKE_FFMPEG_PATH=/opt/homebrew/bin/ffmpeg pnpm smoke:packaged
VIDEOGRE_SMOKE_OUTPUT_DIR=/tmp/videogre-smoke pnpm smoke:packaged
```

## Current macOS Target

- Packaging tool: Electron Builder
- App id: `dev.theorcdev.videogre`
- Product name: `Videogre`
- Primary local target: unsigned macOS app directory
- Local DMG target: unsigned
- Production DMG target: signed and notarized when release secrets are present
- App icon: generated from the current Videogre logo
- FFmpeg: external for alpha; public v1 should bundle an LGPL-compatible build while keeping the Settings override

## Signing And Notarization

Unsigned local builds are useful for smoke testing only. The production release path is:

```sh
pnpm dist:desktop:signed
```

The GitHub Actions workflow at `.github/workflows/release-macos.yml` runs that command for manual dispatches and `v*` tags.

Required GitHub secrets:

- `CSC_LINK`: base64-encoded Developer ID Application certificate archive or a secure URL supported by Electron Builder
- `CSC_KEY_PASSWORD`: certificate archive password
- `APPLE_ID`: Apple Developer account email
- `APPLE_APP_SPECIFIC_PASSWORD`: app-specific password for notarization
- `APPLE_TEAM_ID`: Apple Developer Team ID

A distributable macOS build still needs:

- Apple Developer account and Team ID
- Developer ID Application certificate
- Hardened runtime configuration
- Entitlements review for screen, camera, microphone, and file access
- Notarization credentials in CI or local release environment
- Gatekeeper validation on a clean macOS account

Electron Builder's [macOS docs](https://www.electron.build/docs/mac) describe hardened runtime, entitlements, and notarization requirements. Electron's [code signing guide](https://www.electronjs.org/docs/latest/tutorial/code-signing) explains why distributed macOS apps need signing and notarization.

## FFmpeg Strategy

Decision:

- Alpha/internal builds keep FFmpeg external.
- Public v1 should bundle an LGPL-compatible FFmpeg build and keep the custom FFmpeg path override in Settings.

Rationale:

- External FFmpeg is acceptable while the product is still a technical spike and local alpha.
- Public creator UX should not require Homebrew or manual FFmpeg repair before first recording.
- Keeping a Settings override preserves debugging and power-user workflows.

Do not bundle a GPL or nonfree FFmpeg build unless the product/legal strategy explicitly changes.

Bundling follow-up:

- source an LGPL-compatible macOS universal or per-arch FFmpeg build
- include license notices and source-offer obligations
- add a backend binary resolution path that prefers the bundled FFmpeg and falls back to system/custom paths
- add packaged smoke coverage with the bundled FFmpeg path

## Release Checklist

- `pnpm install`
- `pnpm typecheck`
- `pnpm build`
- `cargo fmt --check --all`
- `cargo test`
- `cargo clippy -- -D warnings`
- `pnpm package:desktop`
- `pnpm smoke:packaged`
- Launch the packaged app from `apps/desktop/release/mac*/Videogre.app`
- Confirm the packaged backend emits `READY`
- Confirm FFmpeg unavailable states are visible and non-crashing
- Record a short MKV with system FFmpeg installed
- Stop recording and confirm the session appears in Library
