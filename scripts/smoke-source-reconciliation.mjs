import assert from 'node:assert/strict'
import { mkdir, readFile, rm, writeFile } from 'node:fs/promises'
import { join } from 'node:path'
import { tmpdir } from 'node:os'
import { createRequire } from 'node:module'

const require = createRequire(import.meta.url)
const ts = require('../apps/desktop/node_modules/typescript')

const sourcePath = join(process.cwd(), 'apps/desktop/src/renderer/src/lib/capture.ts')
const tempDir = join(tmpdir(), `videorc-source-reconciliation-${Date.now()}`)
const tempModule = join(tempDir, 'capture.cjs')

await mkdir(tempDir, { recursive: true })
try {
  const source = await readFile(sourcePath, 'utf8')
  const transpiled = ts.transpileModule(source, {
    compilerOptions: {
      module: ts.ModuleKind.CommonJS,
      target: ts.ScriptTarget.ES2022,
      esModuleInterop: true
    }
  })
  await writeFile(tempModule, transpiled.outputText)

  const storage = new Map()
  globalThis.localStorage = {
    getItem: (key) => storage.get(key) ?? null,
    setItem: (key, value) => storage.set(key, String(value)),
    removeItem: (key) => storage.delete(key),
    clear: () => storage.clear()
  }

  const {
    defaultCaptureConfig,
    loadCaptureConfig,
    persistableCaptureConfig,
    reconcileSourceSelection,
    sourceSelectionChangeMessages,
    STORAGE_KEYS
  } = require(tempModule)
  assert.equal(typeof reconcileSourceSelection, 'function')

  const devices = [
    device('screen:screencapturekit:222', 'Built-in Display', 'screen'),
    device('screen:screencapturekit:333', 'Studio Monitor', 'screen'),
    device('window:screencapturekit:111', 'Editor', 'window'),
    device('camera:avfoundation-native:face-time', 'FaceTime HD Camera', 'camera'),
    device('camera:avfoundation-native:desk', 'Desk Camera', 'camera'),
    device('microphone:coreaudio:podcast', 'Podcast Mic', 'microphone'),
    device('microphone:coreaudio:laptop', 'Laptop Mic', 'microphone')
  ]

  localStorage.setItem(
    STORAGE_KEYS.captureConfig,
    JSON.stringify(
      persistableCaptureConfig({
        ...defaultCaptureConfig,
        sources: {
          screenId: 'screen-old',
          screenName: 'Built-in Display',
          cameraId: 'camera-old',
          cameraName: 'FaceTime HD Camera',
          microphoneId: 'mic-old',
          microphoneName: 'Podcast Mic'
        }
      })
    )
  )
  const loaded = loadCaptureConfig()
  assert.equal(loaded.audio.microphoneSyncOffsetMs, 0)
  assert.deepEqual(loaded.sources, {
    screenId: 'screen-old',
    screenName: 'Built-in Display',
    cameraId: 'camera-old',
    cameraName: 'FaceTime HD Camera',
    microphoneId: 'mic-old',
    microphoneName: 'Podcast Mic',
    testPattern: false
  })
  const reloadedSources = reconcileSourceSelection(loaded.sources, devices)
  assert.deepEqual(reloadedSources, {
    screenId: 'screen:screencapturekit:222',
    screenName: 'Built-in Display',
    windowId: undefined,
    windowName: undefined,
    cameraId: 'camera:avfoundation-native:face-time',
    cameraName: 'FaceTime HD Camera',
    microphoneId: 'microphone:coreaudio:podcast',
    microphoneName: 'Podcast Mic',
    testPattern: false
  })
  assert.deepEqual(sourceSelectionChangeMessages(loaded.sources, reloadedSources), [
    'Capture source "Built-in Display" was restored by name because its system ID changed.',
    'Camera "FaceTime HD Camera" was restored by name because its system ID changed.',
    'Microphone "Podcast Mic" was restored by name because its system ID changed.'
  ])

  localStorage.setItem(
    STORAGE_KEYS.captureConfig,
    JSON.stringify({
      ...defaultCaptureConfig,
      audio: {
        ...defaultCaptureConfig.audio,
        microphoneSyncOffsetMs: -250
      }
    })
  )
  assert.equal(loadCaptureConfig().audio.microphoneSyncOffsetMs, 0)

  localStorage.setItem(
    STORAGE_KEYS.captureConfig,
    JSON.stringify({
      ...defaultCaptureConfig,
      audio: {
        ...defaultCaptureConfig.audio,
        microphoneSyncOffsetMs: -250,
        microphoneSyncOffsetUserSet: true
      }
    })
  )
  assert.equal(loadCaptureConfig().audio.microphoneSyncOffsetMs, -250)

  assert.deepEqual(
    reconcileSourceSelection(
      {
        screenId: 'screen-old',
        screenName: 'Built-in Display',
        cameraId: 'camera-old',
        cameraName: 'FaceTime HD Camera',
        microphoneId: 'mic-old',
        microphoneName: 'Podcast Mic'
      },
      devices
    ),
    {
      screenId: 'screen:screencapturekit:222',
      screenName: 'Built-in Display',
      windowId: undefined,
      windowName: undefined,
      cameraId: 'camera:avfoundation-native:face-time',
      cameraName: 'FaceTime HD Camera',
      microphoneId: 'microphone:coreaudio:podcast',
      microphoneName: 'Podcast Mic'
    }
  )

  assert.deepEqual(
    reconcileSourceSelection(
      {
        windowId: 'window:screencapturekit:111',
        windowName: 'Editor',
        screenId: 'screen-old',
        screenName: 'Built-in Display',
        cameraId: 'camera:avfoundation-native:desk',
        cameraName: 'Desk Camera',
        microphoneId: 'microphone:coreaudio:laptop',
        microphoneName: 'Laptop Mic'
      },
      devices
    ),
    {
      screenId: undefined,
      screenName: undefined,
      windowId: 'window:screencapturekit:111',
      windowName: 'Editor',
      cameraId: 'camera:avfoundation-native:desk',
      cameraName: 'Desk Camera',
      microphoneId: 'microphone:coreaudio:laptop',
      microphoneName: 'Laptop Mic'
    }
  )

  const missingSources = {
    screenId: 'missing-screen',
    screenName: 'Missing Display',
    cameraId: 'missing-camera',
    cameraName: 'Missing Camera',
    microphoneId: 'missing-mic',
    microphoneName: 'Missing Mic'
  }
  assert.deepEqual(reconcileSourceSelection(missingSources, devices), {
    screenId: 'screen:screencapturekit:222',
    screenName: 'Built-in Display',
    windowId: undefined,
    windowName: undefined,
    cameraId: 'camera:avfoundation-native:face-time',
    cameraName: 'FaceTime HD Camera',
    microphoneId: 'microphone:coreaudio:podcast',
    microphoneName: 'Podcast Mic'
  })
  assert.deepEqual(
    sourceSelectionChangeMessages(
      missingSources,
      reconcileSourceSelection(missingSources, devices)
    ),
    [
      'Capture source "Missing Display" is unavailable, so Videorc selected "Built-in Display".',
      'Camera "Missing Camera" is unavailable, so Videorc selected "FaceTime HD Camera".',
      'Microphone "Missing Mic" is unavailable, so Videorc selected "Podcast Mic".'
    ]
  )

  assert.deepEqual(
    reconcileSourceSelection(
      {
        screenId: 'screen:screencapturekit:222',
        screenName: 'Built-in Display',
        cameraId: 'camera:avfoundation-native:face-time',
        cameraName: 'FaceTime HD Camera',
        microphoneId: 'microphone:coreaudio:podcast',
        microphoneName: 'Podcast Mic'
      },
      devices.map((item) => (item.kind === 'camera' ? { ...item, status: 'unavailable' } : item))
    ),
    {
      screenId: 'screen:screencapturekit:222',
      screenName: 'Built-in Display',
      windowId: undefined,
      windowName: undefined,
      cameraId: undefined,
      cameraName: undefined,
      microphoneId: 'microphone:coreaudio:podcast',
      microphoneName: 'Podcast Mic'
    }
  )

  console.log(
    'Source reconciliation smoke OK - persisted IDs, name rematch, and fallback behavior verified.'
  )
} finally {
  await rm(tempDir, { recursive: true, force: true })
}

function device(id, name, kind) {
  return { id, name, kind, status: 'available' }
}
