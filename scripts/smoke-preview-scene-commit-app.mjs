#!/usr/bin/env node
// Preview scene commit smoke: scene edits must advance through backend-owned
// revisions, even after the compositor has a high wallclock/session revision.

import { request as httpRequest } from 'node:http'
import { mkdirSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'

import { launchDevApp } from './lib/app-launcher.mjs'
import { connectBackend, request } from './smoke-recording-session.mjs'

const timeoutMs = Number(process.env.VIDEORC_SMOKE_TIMEOUT_MS ?? 120000)
const settleMs = Number(process.env.VIDEORC_PREVIEW_SCENE_COMMIT_SETTLE_MS ?? 800)
const outputDirectory = join(tmpdir(), `videorc-preview-scene-commit-${Date.now()}`)
mkdirSync(outputDirectory, { recursive: true })

const launched = await launchDevApp({
  requiredMarkers: ['backend-ready', 'preview-motion-ready'],
  timeoutMs,
  env: {
    VIDEORC_SMOKE_OUTPUT_DIR: outputDirectory,
    VIDEORC_SMOKE_PRINT_BACKEND_READY: '1',
    VIDEORC_SMOKE_COMMAND_SERVER: '1',
    VIDEORC_SMOKE_PREVIEW_MOTION: '1',
    VIDEORC_NATIVE_PREVIEW_SURFACE: '1'
  }
})

let ws
try {
  const backend = launched.connections['backend-ready']
  const smoke = launched.connections['preview-motion-ready']
  ws = await connectBackend(backend, timeoutMs)

  await smokeCommand(smoke, 'open-tab', {
    tab: 'studio',
    waitFor: '[data-videorc-preview-card]'
  })
  await smokeCommand(smoke, 'preview-window-open')
  await waitForSurfaceLive(smoke)
  await smokeCommand(smoke, 'enable-synthetic-source', { settleMs })
  await smokeCommand(smoke, 'select-layout-preset', { preset: 'screen-camera', settleMs })
  await sleep(settleMs)

  const beforeScene = await request(ws, timeoutMs, 'scene.get')
  const beforeCompositor = await request(ws, timeoutMs, 'compositor.status')
  const highRevision = Math.max(Number(beforeCompositor.sceneRevision ?? 0), Date.now() + 1_000_000)
  const layout = beforeCompositor.sceneLayout ?? defaultLayout()

  const highStatus = await request(ws, timeoutMs, 'compositor.scene.update', {
    revision: highRevision,
    scene: beforeScene,
    layout,
    activeScreen: null
  })
  if (highStatus.sceneRevision !== highRevision) {
    throw new Error(
      `High revision setup failed: expected ${highRevision}, got ${highStatus.sceneRevision}.`
    )
  }

  const source =
    beforeScene.sources.find((candidate) => candidate.visible) ?? beforeScene.sources[0]
  if (!source) {
    throw new Error(`Scene has no source to mutate: ${JSON.stringify(beforeScene)}`)
  }

  const nextX = Math.min(0.82, Math.max(0.02, Number(source.transform?.x ?? 0) + 0.03))
  const commit = await request(ws, timeoutMs, 'scene.source.transform.update', {
    sourceId: source.id,
    transform: { x: nextX }
  })
  if (typeof commit.sceneRevision !== 'number') {
    throw new Error(
      `Scene transform returned an uncommitted response instead of SceneCommitStatus: ${JSON.stringify(
        commit
      )}`
    )
  }
  if (commit.sceneRevision <= highRevision) {
    throw new Error(
      `Scene transform committed revision ${commit.sceneRevision}, expected > ${highRevision}.`
    )
  }

  const compositor = await waitForCompositorRevision(ws, commit.sceneRevision)
  const surface = await waitForSurfaceRevision(smoke, commit.sceneRevision)
  const scene = await request(ws, timeoutMs, 'scene.get')
  assertCommitResult({ commit, compositor, surface, scene, sourceId: source.id, nextX })

  console.log(
    `Preview scene commit smoke OK - stale revision ${highRevision} advanced to ${commit.sceneRevision}, surface ${surface.sceneRevision}.`
  )
} finally {
  try {
    ws?.close()
  } catch {
    // Best-effort cleanup.
  }
  await launched.stop()
}

async function waitForSurfaceLive(smoke) {
  let last = null
  const deadline = Date.now() + timeoutMs
  while (Date.now() < deadline) {
    last = await smokeCommand(smoke, 'native-preview-surface-status')
    if (last.state === 'live' && last.bounds?.width > 0 && last.bounds?.height > 0) {
      return last
    }
    await sleep(100)
  }
  throw new Error(
    `Timed out waiting for live preview surface. Last status: ${JSON.stringify(last)}`
  )
}

async function waitForCompositorRevision(connection, revision) {
  let last = null
  const deadline = Date.now() + timeoutMs
  while (Date.now() < deadline) {
    last = await request(connection, timeoutMs, 'compositor.status')
    if (last.sceneRevision === revision && last.frameSceneRevision === revision) {
      return last
    }
    await sleep(100)
  }
  throw new Error(
    `Timed out waiting for compositor rendered revision ${revision}. Last status: ${JSON.stringify(
      last
    )}`
  )
}

async function waitForSurfaceRevision(smoke, revision) {
  let last = null
  const deadline = Date.now() + timeoutMs
  while (Date.now() < deadline) {
    last = await smokeCommand(smoke, 'preview-surface-scene-state')
    if (last.sceneRevision === revision) {
      return last
    }
    await sleep(100)
  }
  throw new Error(
    `Timed out waiting for detached preview surface revision ${revision}. Last state: ${JSON.stringify(
      last
    )}`
  )
}

function assertCommitResult({ commit, compositor, surface, scene, sourceId, nextX }) {
  if (compositor.sceneRevision !== commit.sceneRevision) {
    throw new Error(
      `Compositor revision ${compositor.sceneRevision} did not match commit ${commit.sceneRevision}.`
    )
  }
  if (compositor.frameSceneRevision !== commit.sceneRevision) {
    throw new Error(
      `Compositor frame revision ${compositor.frameSceneRevision} did not render commit ${commit.sceneRevision}.`
    )
  }
  if (surface.sceneRevision !== commit.sceneRevision) {
    throw new Error(
      `Preview surface revision ${surface.sceneRevision} did not match commit ${commit.sceneRevision}.`
    )
  }
  const source = scene.sources.find((candidate) => candidate.id === sourceId)
  if (!source) {
    throw new Error(`Committed scene lost source ${sourceId}: ${JSON.stringify(scene)}`)
  }
  if (Math.abs(Number(source.transform?.x ?? 0) - nextX) > 0.0001) {
    throw new Error(
      `Committed scene source ${sourceId} x=${source.transform?.x}, expected ${nextX}.`
    )
  }
  if (!surface.visibleSourceIds?.includes(sourceId)) {
    throw new Error(
      `Preview surface visible sources ${surface.visibleSourceIds?.join(', ') ?? '(none)'} did not include ${sourceId}.`
    )
  }
  if (surface.surfaceStatus?.state !== 'live') {
    throw new Error(`Preview surface is not live: ${JSON.stringify(surface.surfaceStatus)}`)
  }
}

function smokeCommand(smoke, command, params = {}) {
  const body = JSON.stringify({ command, params })
  return new Promise((resolve, reject) => {
    const req = httpRequest(
      {
        hostname: smoke.host,
        port: smoke.port,
        path: '/command',
        method: 'POST',
        headers: {
          'content-type': 'application/json',
          'content-length': Buffer.byteLength(body)
        },
        timeout: timeoutMs
      },
      (res) => {
        let data = ''
        res.setEncoding('utf8')
        res.on('data', (chunk) => {
          data += chunk
        })
        res.on('end', () => {
          try {
            const payload = JSON.parse(data)
            if (res.statusCode !== 200 || payload.error) {
              reject(
                new Error(
                  `Smoke command ${command} failed (${res.statusCode}): ${payload.error ?? data}`
                )
              )
              return
            }
            resolve(payload.result ?? payload)
          } catch (error) {
            reject(error)
          }
        })
      }
    )
    req.on('error', reject)
    req.write(body)
    req.end()
  })
}

function defaultLayout() {
  return {
    layoutPreset: 'screen-camera',
    cameraTransformMode: 'preset',
    cameraTransform: null,
    cameraCorner: 'bottom-right',
    cameraSize: 'medium',
    cameraShape: 'rectangle',
    cameraMargin: 32,
    cameraFit: 'fill',
    cameraMirror: false,
    cameraZoom: 100,
    cameraOffsetX: 0,
    cameraOffsetY: 0,
    sideBySideSplit: '70-30',
    sideBySideCameraSide: 'right'
  }
}

function sleep(ms) {
  return new Promise((resolve) => setTimeout(resolve, ms))
}
