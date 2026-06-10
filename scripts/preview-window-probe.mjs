#!/usr/bin/env node
// Preview window probe — headless verification of the detached preview window
// (UI rewrite U1). This is the DEFAULT preview mode; the glue probe covers the
// legacy embedded stage behind VIDEORC_NATIVE_PREVIEW_EMBEDDED=1.
//
// Verifies on the real pipeline:
//   1. Opening the preview window creates the surface session and the surface
//      covers the window's content rect.
//   2. Moving and resizing the window keep the surface glued to the content rect.
//   3. Closing the window takes the surface off the screen.
//   4. Reopening brings the surface back.
//
// Placement oracles: the `preview-window-state` smoke command (preview window +
// Electron proof surface geometry) and CGWindowList floating-level geometry for
// the native CAMetalLayer helper window. Both work without Screen Recording
// permission.
//
//   node scripts/preview-window-probe.mjs
//
// Exits 0 when all assertions pass, 1 otherwise.

import { spawnSync } from 'node:child_process'
import { mkdirSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'

import { launchDevApp, stopProcess } from './lib/app-launcher.mjs'

const timeoutMs = Number(process.env.VIDEORC_SMOKE_TIMEOUT_MS ?? 180000)
const outputDirectory = join(tmpdir(), `videorc-preview-window-probe-${Date.now()}`)
mkdirSync(outputDirectory, { recursive: true })

let launched
let smoke
const failures = []
let lastWindowDump = []
let exitCode = 0
try {
  exitCode = await main()
} catch (error) {
  console.error(`preview window probe failed: ${error?.message ?? error}`)
  exitCode = 2
} finally {
  if (launched) await stopProcess(launched.process)
}
process.exit(exitCode)

async function main() {
  console.log('Launching dev app for preview window probe…')
  launched = await launchDevApp({
    timeoutMs,
    requiredMarkers: ['backend-ready', 'preview-motion-ready'],
    env: {
      VIDEORC_SMOKE_OUTPUT_DIR: outputDirectory,
      VIDEORC_NATIVE_PREVIEW_SURFACE: '1',
      VIDEORC_DISABLE_AUTO_PREVIEW: '1',
      VIDEORC_SMOKE_COMMAND_SERVER: '1'
    },
    onLine: (line) => console.log(line)
  })
  smoke = launched.connections['preview-motion-ready']

  // --- Open: surface session created at the window's content rect ---------------
  const opened = await smokeCommand('preview-window-open')
  assertProbe(opened.open === true, 'open: preview window reports open', JSON.stringify(opened))
  assertProbe(
    opened.embeddedMode !== true,
    'open: detached mode is the default (no embedded flag)',
    JSON.stringify(opened)
  )

  // Deterministic starting frame: persisted/relative frames drifted off-screen
  // across runs and macOS clamping broke the geometry asserts.
  await smokeCommand('preview-window-set-bounds', { x: 240, y: 160, width: 960, height: 568 })
  let state = await waitForSurfaceAtContentRect('open: surface covers the preview window content rect')

  // --- Move: surface follows -----------------------------------------------------
  await smokeCommand('preview-window-set-bounds', { x: 364, y: 246 })
  state = await waitForSurfaceAtContentRect('move: surface follows the preview window')

  // --- Resize: surface follows ----------------------------------------------------
  await smokeCommand('preview-window-set-bounds', { width: 720, height: 460 })
  state = await waitForSurfaceAtContentRect('resize: surface matches the resized content rect')
  assertProbe(
    Math.abs(state.contentBounds.width - 720) <= 4,
    'resize: content width tracks the requested window width',
    JSON.stringify(state.contentBounds)
  )

  // --- Close: surface leaves the screen AND the session tears down (U2) -----------
  const sizeHint = { width: state.contentBounds.width, height: state.contentBounds.height }
  await smokeCommand('preview-window-close')
  await assertSurfaceHidden('close: surface leaves the screen with the window', sizeHint)
  const closedState = await waitFor(
    async () => smokeCommand('preview-window-state'),
    (s) => s.open === false && s.surface.exists === false && s.framePollingSuppressedFlag === true,
    8000
  )
  assertProbe(closedState.ok, 'close: surface session destroyed and frame polling suppressed', JSON.stringify(closedState.last))
  const decayed = await waitFor(
    async () => smokeCommand('preview-window-state'),
    (s) => s.nativeOwnsPlacement === false,
    4000
  )
  assertProbe(decayed.ok, 'close: native presents stop (placement authority decays)', JSON.stringify(decayed.last))

  // --- Reopen: surface returns and polling resumes ---------------------------------
  await smokeCommand('preview-window-open')
  await waitForSurfaceAtContentRect('reopen: surface returns at the window content rect')
  const reopened = await waitFor(
    async () => smokeCommand('preview-window-state'),
    (s) => s.framePollingSuppressedFlag === false,
    5000
  )
  assertProbe(reopened.ok, 'reopen: frame polling resumes', JSON.stringify(reopened.last))

  console.log('\n=== Preview window probe summary ===')
  if (failures.length === 0) {
    console.log('PASS — open, move, resize, close, and reopen all keep the surface glued to the preview window.')
    return 0
  }
  for (const failure of failures) console.log(`FAIL: ${failure}`)
  return 1
}

/**
 * Poll preview-window-state until the surface (proof window, or native helper
 * window when it owns placement) sits on the preview window's content rect.
 */
async function waitForSurfaceAtContentRect(label, tolerance = 6, timeoutMsLocal = 15000) {
  // The surface hides while the app is unfocused (by design); anything stealing
  // focus from the headless app mid-probe (terminals, overlays) must not read as
  // a placement failure. preview-window-open is idempotent and re-focuses.
  await smokeCommand('preview-window-open')
  const deadline = Date.now() + timeoutMsLocal
  let state = null
  do {
    state = await smokeCommand('preview-window-state')
    const expected = state.contentBounds
    if (state.open && expected) {
      const match = (bounds) =>
        bounds &&
        Math.abs(bounds.x - expected.x) <= tolerance &&
        Math.abs(bounds.y - expected.y) <= tolerance &&
        Math.abs(bounds.width - expected.width) <= tolerance &&
        Math.abs(bounds.height - expected.height) <= tolerance
      if (state.surface.visible && match(state.surface.bounds)) {
        assertProbe(true, `${label} [proof-window]`, '')
        return state
      }
      if (state.nativeOwnsPlacement) {
        // Detached mode runs the helper window at NORMAL level (it stacks with
        // the preview window as one app), so match by owner, not layer.
        const native = windowList().find((w) => w.owner === 'native_preview_host_helper' && match(w))
        if (native) {
          assertProbe(
            native.layer === 0,
            `${label} [native-window at normal level]`,
            `helper window layer ${native.layer} — floating level means it covers every app`
          )
          return state
        }
      }
    }
    await sleep(250)
  } while (Date.now() < deadline)
  assertProbe(
    false,
    label,
    `state: ${JSON.stringify(state)}, floating: ${JSON.stringify(lastWindowDump.filter((w) => w.layer >= 3))}`
  )
  return state
}

async function assertSurfaceHidden(label, sizeHint, timeoutMsLocal = 8000) {
  const deadline = Date.now() + timeoutMsLocal
  let state = null
  let floating = []
  do {
    state = await smokeCommand('preview-window-state')
    floating = windowList().filter(
      (w) =>
        w.owner === 'native_preview_host_helper' &&
        Math.abs(w.width - sizeHint.width) <= 8 &&
        Math.abs(w.height - sizeHint.height) <= 8
    )
    if (state.surface.visible === false && floating.length === 0) {
      assertProbe(true, label, '')
      return
    }
    await sleep(250)
  } while (Date.now() < deadline)
  assertProbe(false, label, `state: ${JSON.stringify(state)}, floating: ${JSON.stringify(floating)}`)
}

async function waitFor(fetchState, predicate, timeoutMsLocal) {
  const deadline = Date.now() + timeoutMsLocal
  let last = null
  do {
    last = await fetchState()
    if (predicate(last)) {
      return { ok: true, last }
    }
    await sleep(250)
  } while (Date.now() < deadline)
  return { ok: false, last }
}

async function smokeCommand(command, params = {}) {
  const response = await fetch(`http://${smoke.host}:${smoke.port}/command`, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ command, params })
  })
  const payload = await response.json()
  if (!response.ok || !payload.ok) {
    throw new Error(payload?.error ?? `${command} smoke command failed`)
  }
  return payload.result
}

function windowList() {
  const swift = `
import CoreGraphics
import Foundation
let list = CGWindowListCopyWindowInfo([.optionOnScreenOnly], kCGNullWindowID) as! [[String: Any]]
for w in list {
  let pid = w[kCGWindowOwnerPID as String] as? Int ?? 0
  let owner = w[kCGWindowOwnerName as String] as? String ?? ""
  let layer = w[kCGWindowLayer as String] as? Int ?? 0
  let b = w[kCGWindowBounds as String] as? [String: Double] ?? [:]
  print("\\(pid)\\t\\(owner)\\t\\(layer)\\t\\(b["X"] ?? -1)\\t\\(b["Y"] ?? -1)\\t\\(b["Width"] ?? -1)\\t\\(b["Height"] ?? -1)")
}
`
  const file = join(outputDirectory, 'windows.swift')
  writeFileSync(file, swift)
  const result = spawnSync('swift', [file], { encoding: 'utf8', timeout: 60000 })
  if (result.status !== 0) {
    throw new Error(`window list probe failed: ${result.stderr?.slice(0, 400)}`)
  }
  lastWindowDump = result.stdout
    .trim()
    .split('\n')
    .filter(Boolean)
    .map((line) => {
      const [pid, owner, layer, x, y, width, height] = line.split('\t')
      return {
        pid: Number(pid),
        owner,
        layer: Number(layer),
        x: Number(x),
        y: Number(y),
        width: Number(width),
        height: Number(height)
      }
    })
  return lastWindowDump
}

function assertProbe(condition, label, detail) {
  if (condition) {
    console.log(`OK   ${label}`)
  } else {
    console.log(`FAIL ${label} — ${detail}`)
    failures.push(`${label} — ${detail}`)
  }
}

function sleep(ms) {
  return new Promise((resolve) => setTimeout(resolve, ms))
}
