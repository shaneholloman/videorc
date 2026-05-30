# Videogre Distribution Notes

Status: first packaging foundation for local macOS app bundles.

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

## Current macOS Target

- Packaging tool: Electron Builder
- App id: `dev.theorcdev.videogre`
- Product name: `Videogre`
- Primary local target: unsigned macOS app directory
- DMG target: configured, but still unsigned and not notarized
- App icon: generated from the current Videogre logo
- FFmpeg: remains an external system dependency for this phase

## Signing And Notarization

Unsigned local builds are useful for smoke testing only. A distributable macOS build still needs:

- Apple Developer account and Team ID
- Developer ID Application certificate
- Hardened runtime configuration
- Entitlements review for screen, camera, microphone, and file access
- Notarization credentials in CI or local release environment
- Gatekeeper validation on a clean macOS account

## FFmpeg Strategy

The app currently expects a system FFmpeg binary, either on `PATH` or configured in Settings. Before public distribution, choose one of:

- keep FFmpeg external and add first-run install guidance
- bundle an LGPL-compatible FFmpeg build with source/notice obligations documented
- support both, preferring bundled FFmpeg and allowing a custom override

Do not bundle a GPL or nonfree FFmpeg build unless the product/legal strategy explicitly changes.

## Release Checklist

- `pnpm install`
- `pnpm typecheck`
- `pnpm build`
- `cargo fmt --check --all`
- `cargo test`
- `cargo clippy -- -D warnings`
- `pnpm package:desktop`
- Launch the packaged app from `apps/desktop/release/mac*/Videogre.app`
- Confirm the packaged backend emits `READY`
- Confirm FFmpeg unavailable states are visible and non-crashing
- Record a short MKV with system FFmpeg installed
- Stop recording and confirm the session appears in Library
