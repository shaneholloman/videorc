import { spawn } from 'node:child_process'
import { existsSync, mkdirSync, statSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { dirname, join, resolve } from 'node:path'

import { connectBackend, request } from './smoke-recording-session.mjs'

const repoRoot = resolve(import.meta.dirname, '..')
const outputDirectory = resolve(
  process.env.VIDEORC_SMOKE_OUTPUT_DIR ?? join(tmpdir(), `videorc-recording-performance-${Date.now()}`)
)
const ffmpegPath = process.env.VIDEORC_SMOKE_FFMPEG_PATH ?? 'ffmpeg'
const ffprobePath = process.env.VIDEORC_SMOKE_FFPROBE_PATH ?? resolveSiblingFfprobe(ffmpegPath) ?? 'ffprobe'
const timeoutMs = Number(process.env.VIDEORC_SMOKE_TIMEOUT_MS ?? 120000)
const recordingMs = Number(process.env.VIDEORC_PERF_RECORDING_MS ?? 20000)
const warmupMs = Number(process.env.VIDEORC_PERF_WARMUP_MS ?? 12000)
const previewPollMs = Number(process.env.VIDEORC_PERF_PREVIEW_POLL_MS ?? 250)
const minSpeed = Number(process.env.VIDEORC_PERF_MIN_SPEED ?? 0.98)
const maxSkewMs = Number(process.env.VIDEORC_PERF_MAX_AV_SKEW_MS ?? 250)

const scenarios = [
  { label: '1440p30', width: 2560, height: 1440, fps: 30, bitrateKbps: 8000 },
  { label: '1080p30', width: 1920, height: 1080, fps: 30, bitrateKbps: 6000 }
]

let appProcess
let stopping = false

mkdirSync(outputDirectory, { recursive: true })

try {
  const connection = await launchAndReadConnection()
  await runPerformanceSmoke(connection)
} finally {
  await stopApp()
}

async function runPerformanceSmoke(connection) {
  const ws = await connectBackend(connection, timeoutMs)
  const samples = []
  try {
    ws.addEventListener('message', (event) => {
      try {
        const message = JSON.parse(event.data)
        if (message.event === 'diagnostics.stats') {
          samples.push({ ...message.payload, receivedAt: Date.now() })
        }
      } catch {
        // Ignore non-JSON noise from the socket.
      }
    })

    const health = await request(ws, timeoutMs, 'health.ping', { ffmpegPath })
    if (!health?.ffmpeg?.available) {
      throw new Error(health?.ffmpeg?.message ?? 'FFmpeg is unavailable for performance smoke.')
    }
    await assertFfprobeAvailable()
    console.log(`Recording performance smoke using FFmpeg: ${ffmpegPath}`)
    console.log(`Recording performance smoke using FFprobe: ${ffprobePath}`)

    for (const scenario of scenarios) {
      await runScenario(ws, connection, samples, scenario)
    }
  } finally {
    ws.close()
  }
}

async function runScenario(ws, connection, samples, scenario) {
  samples.length = 0
  await request(ws, timeoutMs, 'preview.live.start', previewParams(scenario))

  const scenarioStartedAt = Date.now()
  const started = await request(ws, timeoutMs, 'session.start', sessionParams(scenario))
  if (started.state !== 'recording') {
    throw new Error(`[${scenario.label}] Expected recording state after start, got ${started.state}.`)
  }

  const stopPreviewPolling = pollPreviewFrames(connection)
  await sleep(recordingMs)
  stopPreviewPolling()

  const stopped = await request(ws, timeoutMs, 'session.stop')
  const outputPath = stopped.outputPath ?? started.outputPath
  if (!outputPath || !existsSync(outputPath)) {
    throw new Error(`[${scenario.label}] Recording output was not created: ${outputPath ?? 'missing path'}`)
  }
  const size = statSync(outputPath).size
  if (size <= 0) {
    throw new Error(`[${scenario.label}] Recording output is empty: ${outputPath}`)
  }

  const stats = summarizeDiagnostics(samples, scenario.fps, scenarioStartedAt)
  assertStatsHealthy(scenario, stats)
  const skew = await audioVideoSkewMs(outputPath)
  if (skew > maxSkewMs) {
    throw new Error(`[${scenario.label}] Audio/video duration skew ${skew.toFixed(1)}ms exceeded ${maxSkewMs}ms.`)
  }

  console.log(
    `Recording performance [${scenario.label}] OK: ${outputPath} (${size} bytes), min speed ${format(stats.minSpeed)}x, min FPS ${format(stats.minFps)}, A/V skew ${skew.toFixed(1)}ms, maintenance samples ${stats.maintenanceSamples}, duplicate samples ${stats.duplicateCaptureSamples}, max RSS ${formatBytes(stats.maxBackendRssBytes)}, max FFmpeg procs ${stats.maxActiveFfmpegProcesses}, max FFprobe procs ${stats.maxActiveFfprobeProcesses}`
  )
}

function sessionParams(scenario) {
  return {
    sources: { testPattern: true },
    layout: {
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
    },
    output: {
      recordEnabled: true,
      streamEnabled: false,
      outputDirectory,
      ffmpegPath,
      video: {
        preset: 'custom',
        width: scenario.width,
        height: scenario.height,
        fps: scenario.fps,
        bitrateKbps: scenario.bitrateKbps
      },
      rtmp: {
        preset: 'custom',
        serverUrl: '',
        streamKey: ''
      }
    },
    audio: {
      microphoneGainDb: 0,
      microphoneMuted: false,
      microphoneSyncOffsetMs: 0
    }
  }
}

function previewParams(scenario) {
  const { sources, layout, output } = sessionParams(scenario)
  return { sources, layout, ffmpegPath, video: output.video }
}

function pollPreviewFrames(connection) {
  let stopped = false
  async function poll() {
    while (!stopped) {
      const url = `http://${connection.host}:${connection.port}/preview/live.jpg?token=${encodeURIComponent(connection.token)}&t=${Date.now()}`
      try {
        await fetch(url)
      } catch {
        // Preview is expendable in this smoke; diagnostics are the assertion surface.
      }
      await sleep(previewPollMs)
    }
  }
  void poll()
  return () => {
    stopped = true
  }
}

function summarizeDiagnostics(samples, targetFps, scenarioStartedAt) {
  const numeric = (value) => (typeof value === 'number' && Number.isFinite(value) ? value : null)
  const steadySamples = samples.filter((sample) => (sample.receivedAt ?? 0) - scenarioStartedAt >= warmupMs)
  const measuredSamples = steadySamples.length ? steadySamples : samples
  const fpsValues = measuredSamples
    .flatMap((sample) => [numeric(sample.captureFps), numeric(sample.renderFps)])
    .filter((value) => value !== null)
  const speedValues = measuredSamples.map((sample) => numeric(sample.encoderSpeed)).filter((value) => value !== null)
  const backendRssValues = measuredSamples
    .map((sample) => numeric(sample.backendRssBytes))
    .filter((value) => value !== null)
  const ffmpegProcessValues = measuredSamples
    .map((sample) => numeric(sample.activeFfmpegProcesses))
    .filter((value) => value !== null)
  const ffprobeProcessValues = measuredSamples
    .map((sample) => numeric(sample.activeFfprobeProcesses))
    .filter((value) => value !== null)
  return {
    minFps: fpsValues.length ? Math.min(...fpsValues) : null,
    minSpeed: speedValues.length ? Math.min(...speedValues) : null,
    droppedFrames: Math.max(0, ...measuredSamples.map((sample) => sample.droppedFrames ?? 0)),
    micDroppedFrames: Math.max(0, ...measuredSamples.map((sample) => sample.micDroppedFrames ?? 0)),
    previewDroppedFrames: Math.max(0, ...measuredSamples.map((sample) => sample.previewDroppedFrames ?? 0)),
    maintenanceSamples: measuredSamples.filter((sample) => sample.ffmpegMaintenanceRunning).length,
    maintenanceCancelSamples: measuredSamples.filter((sample) => sample.ffmpegMaintenanceCancelRequested).length,
    duplicateCaptureSamples: measuredSamples.filter((sample) => Array.isArray(sample.duplicateCaptureSources) && sample.duplicateCaptureSources.length > 0).length,
    maxBackendRssBytes: backendRssValues.length ? Math.max(...backendRssValues) : null,
    maxActiveFfmpegProcesses: ffmpegProcessValues.length ? Math.max(...ffmpegProcessValues) : 0,
    maxActiveFfprobeProcesses: ffprobeProcessValues.length ? Math.max(...ffprobeProcessValues) : 0,
    steadySamples: steadySamples.length,
    targetFps
  }
}

function assertStatsHealthy(scenario, stats) {
  if (stats.minSpeed === null) {
    throw new Error(`[${scenario.label}] No encoder speed diagnostics were captured after ${warmupMs}ms warm-up.`)
  }
  if (stats.minSpeed < minSpeed) {
    throw new Error(`[${scenario.label}] Encoder speed ${format(stats.minSpeed)}x fell below ${minSpeed}x.`)
  }
  if (stats.minFps === null) {
    throw new Error(`[${scenario.label}] No FPS diagnostics were captured after ${warmupMs}ms warm-up.`)
  }
  const minFps = scenario.fps * 0.9
  if (stats.minFps < minFps) {
    throw new Error(`[${scenario.label}] FPS ${format(stats.minFps)} fell below ${format(minFps)}.`)
  }
  if (stats.droppedFrames > 0) {
    throw new Error(`[${scenario.label}] FFmpeg reported ${stats.droppedFrames} dropped frame(s).`)
  }
  if (stats.micDroppedFrames > 0) {
    throw new Error(`[${scenario.label}] Native microphone reported ${stats.micDroppedFrames} dropped frame(s).`)
  }
  if (stats.maintenanceSamples > 0) {
    throw new Error(`[${scenario.label}] Recording overlapped ${stats.maintenanceSamples} maintenance FFmpeg sample(s).`)
  }
  if (stats.duplicateCaptureSamples > 0) {
    throw new Error(`[${scenario.label}] Recording reported ${stats.duplicateCaptureSamples} duplicate capture diagnostic sample(s).`)
  }
}

async function audioVideoSkewMs(outputPath) {
  const probe = await run(ffprobePath, [
    '-v',
    'error',
    '-show_entries',
    'stream=codec_type,duration',
    '-show_entries',
    'format=duration',
    '-of',
    'json',
    outputPath
  ])
  if (probe.status !== 0) {
    throw new Error(`ffprobe failed for ${outputPath}: ${probe.stderr.trim()}`)
  }
  const parsed = JSON.parse(probe.stdout)
  const formatDuration = Number(parsed.format?.duration)
  const durations = new Map()
  for (const stream of parsed.streams ?? []) {
    const duration = Number(stream.duration)
    if (Number.isFinite(duration)) {
      durations.set(stream.codec_type, duration)
    }
  }
  const video = durations.get('video') ?? formatDuration
  const audio = durations.get('audio') ?? formatDuration
  if (!Number.isFinite(video) || !Number.isFinite(audio)) {
    throw new Error(`Could not read audio/video durations from ${outputPath}.`)
  }
  return Math.abs(video - audio) * 1000
}

async function assertFfprobeAvailable() {
  try {
    const result = await run(ffprobePath, ['-version'])
    if (result.status !== 0) {
      throw new Error(result.stderr.trim() || `exit ${result.status}`)
    }
  } catch (error) {
    throw new Error(`FFprobe is required for A/V skew checks. Set VIDEORC_SMOKE_FFPROBE_PATH. ${error.message}`)
  }
}

function run(command, args) {
  return new Promise((resolveRun, rejectRun) => {
    const child = spawn(command, args, { cwd: repoRoot })
    let stdout = ''
    let stderr = ''
    child.stdout.setEncoding('utf8')
    child.stderr.setEncoding('utf8')
    child.stdout.on('data', (text) => {
      stdout += text
    })
    child.stderr.on('data', (text) => {
      stderr += text
    })
    child.on('error', rejectRun)
    child.on('exit', (code) => resolveRun({ status: code ?? 1, stdout, stderr }))
  })
}

function launchAndReadConnection() {
  return new Promise((resolveConnection, rejectConnection) => {
    const timer = setTimeout(() => {
      rejectConnection(new Error(`Timed out waiting for dev backend READY after ${timeoutMs}ms.`))
    }, timeoutMs)

    appProcess = spawn('pnpm', ['dev'], {
      cwd: repoRoot,
      detached: true,
      env: {
        ...process.env,
        VIDEORC_SMOKE_PRINT_BACKEND_READY: '1'
      },
      stdio: ['ignore', 'pipe', 'pipe']
    })

    appProcess.stdout.setEncoding('utf8')
    appProcess.stderr.setEncoding('utf8')
    appProcess.stdout.on('data', (text) => handleAppOutput(text, resolveConnection, timer))
    appProcess.stderr.on('data', (text) => handleAppOutput(text, resolveConnection, timer))
    appProcess.on('error', (error) => {
      clearTimeout(timer)
      rejectConnection(error)
    })
    appProcess.on('exit', (code, signal) => {
      clearTimeout(timer)
      rejectConnection(new Error(`Dev app exited before smoke test completed: code=${code} signal=${signal}`))
    })
  })
}

function handleAppOutput(text, resolveConnection, timer) {
  for (const line of text.split(/\r?\n/)) {
    if (line.trim() && !stopping) {
      console.log(line)
    }

    const marker = '[smoke] backend-ready '
    const index = line.indexOf(marker)
    if (index === -1) {
      continue
    }

    clearTimeout(timer)
    resolveConnection(JSON.parse(line.slice(index + marker.length)))
  }
}

function stopApp() {
  return new Promise((resolveStop) => {
    if (!appProcess?.pid || appProcess.killed) {
      resolveStop()
      return
    }

    const timer = setTimeout(() => {
      killApp('SIGKILL')
      resolveStop()
    }, 5000)

    stopping = true
    appProcess.once('exit', () => {
      clearTimeout(timer)
      resolveStop()
    })
    killApp('SIGTERM')
  })
}

function killApp(signal) {
  if (!appProcess?.pid) {
    return
  }

  try {
    process.kill(-appProcess.pid, signal)
  } catch {
    appProcess.kill(signal)
  }
}

function resolveSiblingFfprobe(path) {
  if (!path.includes('/')) {
    return null
  }
  const candidate = join(dirname(path), 'ffprobe')
  return existsSync(candidate) ? candidate : null
}

function sleep(ms) {
  return new Promise((resolveSleep) => setTimeout(resolveSleep, ms))
}

function format(value) {
  return typeof value === 'number' ? value.toFixed(2) : 'n/a'
}

function formatBytes(value) {
  if (typeof value !== 'number') {
    return 'n/a'
  }
  if (value < 1024 * 1024) {
    return `${(value / 1024).toFixed(0)}KiB`
  }
  return `${(value / (1024 * 1024)).toFixed(1)}MiB`
}
