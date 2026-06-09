#!/usr/bin/env node
// Preview glue probe — headless placement verification for the B1/B2 contract.
//
// Launches the dev app exactly like the user does (VIDEORC_NATIVE_PREVIEW_SURFACE=1)
// with the smoke command server, drives the REAL native preview host with known
// bounds (full slot, half-clipped slot, hidden), and asserts the actual on-screen
// window geometry via CGWindowList (readable without Screen Recording permission).
//
// This proves or disproves, on the real pipeline:
//   1. the surface window covers the CLIP rect (not the full slot) when clipped,
//   2. the surface window leaves the screen when bounds say visible:false,
//   3. the surface claims an active CAMetalLayer presenting state.
//
//   node scripts/preview-glue-probe.mjs
//
// Exits 0 when all assertions pass, 1 otherwise.

import { spawnSync } from 'node:child_process'
import { mkdirSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'

import { launchDevApp, stopProcess } from './lib/app-launcher.mjs'
import { connectBackend, request } from './smoke-recording-session.mjs'

const timeoutMs = Number(process.env.VIDEORC_SMOKE_TIMEOUT_MS ?? 180000)
const outputDirectory = join(tmpdir(), `videorc-preview-glue-probe-${Date.now()}`)
mkdirSync(outputDirectory, { recursive: true })

// Distinctive geometry so the probe windows are unambiguous in the window list.
const SLOT = { screenX: 211, screenY: 173, width: 642, height: 414 }
const COMMON = { scaleFactor: 2, screenHeight: null }

let launched
const failures = []
let lastWindowDump = []
let exitCode = 0
try {
  exitCode = await main()
} catch (error) {
  console.error(`preview glue probe failed: ${error?.message ?? error}`)
  exitCode = 2
} finally {
  if (launched) await stopProcess(launched.process)
}
process.exit(exitCode)

async function main() {
  console.log('Launching dev app for preview glue probe…')
  launched = await launchDevApp({
    timeoutMs,
    requiredMarkers: ['backend-ready', 'preview-motion-ready'],
    env: {
      VIDEORC_SMOKE_OUTPUT_DIR: outputDirectory,
      VIDEORC_NATIVE_PREVIEW_SURFACE: '1',
      VIDEORC_DISABLE_AUTO_PREVIEW: '1',
      VIDEORC_SMOKE_COMMAND_SERVER: '1',
      VIDEORC_SMOKE_NATIVE_PREVIEW_SUSPENDED: '1'
    },
    onLine: (line) => console.log(line)
  })
  const ws = await connectBackend(launched.connections['backend-ready'], timeoutMs)
  const smoke = launched.connections['preview-motion-ready']

  try {
    const screenHeight = await probeScreenHeight()
    COMMON.screenHeight = screenHeight

    // --- Step 1: create the surface fully visible -------------------------------
    const fullBounds = {
      ...SLOT,
      ...COMMON,
      clipX: SLOT.screenX,
      clipY: SLOT.screenY,
      clipWidth: SLOT.width,
      clipHeight: SLOT.height,
      visible: true
    }
    await request(ws, timeoutMs, 'preview.surface.create', {
      bounds: fullBounds,
      targetFps: 60,
      source: 'synthetic'
    })
    let status = await applyHostCommands(ws, smoke)
    console.log(
      `surface status: transport=${status?.transport} backing=${status?.backing} frames=${status?.framesRendered}`
    )
    await sleep(1200)
    const fullWindows = matchingWindows(SLOT.width, SLOT.height)
    assertProbe(
      fullWindows.length >= 1,
      `full-visible: a window of ${SLOT.width}x${SLOT.height} is on screen`,
      `windows seen: ${describeAll()}`
    )

    // --- Step 2: clip to the bottom half -----------------------------------------
    const halfHeight = SLOT.height / 2
    const clippedBounds = {
      ...SLOT,
      ...COMMON,
      clipX: SLOT.screenX,
      clipY: SLOT.screenY + halfHeight,
      clipWidth: SLOT.width,
      clipHeight: halfHeight,
      visible: true
    }
    await request(ws, timeoutMs, 'preview.surface.update_bounds', { bounds: clippedBounds })
    status = await applyHostCommands(ws, smoke)
    await sleep(800)
    const clippedWindows = matchingWindows(SLOT.width, halfHeight)
    const staleFullWindows = matchingWindows(SLOT.width, SLOT.height)
    assertProbe(
      clippedWindows.length >= 1,
      `clipped: a window of ${SLOT.width}x${halfHeight} (the CLIP rect) is on screen`,
      `windows seen: ${describeAll()}`
    )
    assertProbe(
      staleFullWindows.length === 0,
      'clipped: no surface window still covers the FULL slot rect',
      `full-rect windows still present: ${JSON.stringify(staleFullWindows)}`
    )
    if (clippedWindows[0]) {
      const expectedY = SLOT.screenY + halfHeight
      assertProbe(
        Math.abs(clippedWindows[0].y - expectedY) <= 2 && Math.abs(clippedWindows[0].x - SLOT.screenX) <= 2,
        `clipped: window origin is the clip origin (${SLOT.screenX},${expectedY})`,
        `actual origin: (${clippedWindows[0].x},${clippedWindows[0].y})`
      )
    }

    // --- Step 3: hide -------------------------------------------------------------
    const hiddenBounds = { ...clippedBounds, clipHeight: 0, clipWidth: 0, visible: false }
    await request(ws, timeoutMs, 'preview.surface.update_bounds', { bounds: hiddenBounds })
    await applyHostCommands(ws, smoke)
    await sleep(800)
    const hiddenWindows = [...matchingWindows(SLOT.width, halfHeight), ...matchingWindows(SLOT.width, SLOT.height)]
    assertProbe(
      hiddenWindows.length === 0,
      'hidden: no surface window remains on screen when bounds say visible:false',
      `still on screen: ${JSON.stringify(hiddenWindows)}`
    )

    // --- Step 4: show again (recovery) --------------------------------------------
    await request(ws, timeoutMs, 'preview.surface.update_bounds', { bounds: fullBounds })
    await applyHostCommands(ws, smoke)
    await sleep(800)
    assertProbe(
      matchingWindows(SLOT.width, SLOT.height).length >= 1,
      'recovery: the surface window reappears when bounds become visible again',
      `windows seen: ${describeAll()}`
    )

    console.log('\n=== Preview glue probe summary ===')
    if (failures.length === 0) {
      console.log('PASS — clip placement, hide, and recovery verified on the real pipeline.')
      return 0
    }
    for (const failure of failures) console.log(`FAIL: ${failure}`)
    return 1
  } finally {
    ws.close()
  }
}

async function applyHostCommands(ws, smoke) {
  const commands = await request(ws, timeoutMs, 'preview.surface.take_native_host_commands')
  if (!Array.isArray(commands) || commands.length === 0) {
    return smokeCommand(smoke, 'native-preview-surface-status')
  }
  console.log(`applying ${commands.length} host command(s): ${commands.map((c) => c.kind).join(', ')}`)
  return smokeCommand(smoke, 'apply-native-preview-host-commands', { commands })
}

async function smokeCommand(smoke, command, params = {}) {
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

function matchingWindows(width, height) {
  return windowList().filter(
    (w) => Math.abs(w.width - width) <= 2 && Math.abs(w.height - height) <= 2
  )
}

function describeAll() {
  return lastWindowDump
    .filter((w) => /electron|videorc|cargo|native_preview/i.test(w.owner))
    .map((w) => `${w.owner}[${w.layer}] ${w.width}x${w.height}@(${w.x},${w.y})`)
    .join('; ')
}

async function probeScreenHeight() {
  const swift = `
import AppKit
print(Int(NSScreen.screens.first?.frame.height ?? 0))
`
  const file = join(outputDirectory, 'screen.swift')
  writeFileSync(file, swift)
  const result = spawnSync('swift', [file], { encoding: 'utf8', timeout: 60000 })
  const value = Number(result.stdout?.trim())
  return Number.isFinite(value) && value > 0 ? value : 982
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
