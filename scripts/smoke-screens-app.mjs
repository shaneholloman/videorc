import { spawn, spawnSync } from 'node:child_process'
import { existsSync, mkdirSync, statSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join, resolve } from 'node:path'

import { connectBackend, request } from './smoke-recording-session.mjs'

const repoRoot = resolve(import.meta.dirname, '..')
const outputDirectory = resolve(
  process.env.VIDEORC_SMOKE_OUTPUT_DIR ?? join(tmpdir(), `videorc-screens-smoke-${Date.now()}`)
)
const ffmpegPath = process.env.VIDEORC_SMOKE_FFMPEG_PATH ?? 'ffmpeg'
const timeoutMs = Number(process.env.VIDEORC_SMOKE_TIMEOUT_MS ?? 90000)

mkdirSync(outputDirectory, { recursive: true })

let appProcess
let stopping = false

try {
  const redPath = join(outputDirectory, 'screen-red.png')
  const greenPath = join(outputDirectory, 'screen-green.png')
  createSolidPng('red', redPath)
  createSolidPng('lime', greenPath)

  const connection = await launchAndReadConnection()
  const ws = await connectBackend(connection, timeoutMs)
  const statuses = []
  ws.addEventListener('message', (event) => {
    try {
      const message = JSON.parse(event.data)
      if (message?.event === 'recording.status') {
        statuses.push(message.payload)
      }
    } catch {
      // Ignore unrelated smoke output.
    }
  })

  let redScreen
  let greenScreen
  try {
    const health = await request(ws, timeoutMs, 'health.ping', { ffmpegPath })
    if (!health?.ffmpeg?.available) {
      throw new Error(health?.ffmpeg?.message ?? 'FFmpeg is unavailable for Screens smoke.')
    }
    console.log(`Screens smoke using FFmpeg: ${ffmpegPath}`)

    await request(ws, timeoutMs, 'screens.clear')
    redScreen = await request(ws, timeoutMs, 'screens.importImage', { path: redPath, ffmpegPath })
    greenScreen = await request(ws, timeoutMs, 'screens.importImage', { path: greenPath, ffmpegPath })

    await request(ws, timeoutMs, 'screens.activate', { screenId: redScreen.id })
    const started = await request(ws, timeoutMs, 'session.start', sessionParams())
    if (!['recording', 'streaming'].includes(started.state)) {
      throw new Error(`Expected recording state after start, got ${started.state}.`)
    }
    const sessionId = started.sessionId

    await sleep(1200)
    await request(ws, timeoutMs, 'screens.activate', { screenId: greenScreen.id })
    await assertSameRunningSession(ws, sessionId)

    await sleep(1200)
    await request(ws, timeoutMs, 'screens.clear')
    await assertSameRunningSession(ws, sessionId)

    await sleep(1200)
    const stopped = await request(ws, timeoutMs, 'session.stop')
    const outputPath = stopped.outputPath ?? started.outputPath
    verifyOutput(outputPath, statuses, sessionId)
  } finally {
    if (redScreen?.id) {
      await request(ws, timeoutMs, 'screens.delete', { screenId: redScreen.id }).catch(() => {})
    }
    if (greenScreen?.id) {
      await request(ws, timeoutMs, 'screens.delete', { screenId: greenScreen.id }).catch(() => {})
    }
    await request(ws, timeoutMs, 'screens.clear').catch(() => {})
    ws.close()
  }
} finally {
  await stopApp()
}

function createSolidPng(color, outputPath) {
  const result = spawnSync(
    ffmpegPath,
    [
      '-y',
      '-hide_banner',
      '-loglevel',
      'error',
      '-f',
      'lavfi',
      '-i',
      `color=c=${color}:s=640x360`,
      '-frames:v',
      '1',
      outputPath
    ],
    { cwd: repoRoot, encoding: 'utf8' }
  )
  if (result.status !== 0) {
    throw new Error(`Could not create ${color} Screen image: ${result.stderr || result.stdout}`)
  }
}

function sampleRgb(outputPath, seconds) {
  const result = spawnSync(
    ffmpegPath,
    [
      '-hide_banner',
      '-loglevel',
      'error',
      '-ss',
      seconds.toFixed(2),
      '-i',
      outputPath,
      '-frames:v',
      '1',
      '-vf',
      'scale=1:1,format=rgb24',
      '-f',
      'rawvideo',
      'pipe:1'
    ],
    { encoding: null }
  )
  if (result.status !== 0 || result.stdout.length < 3) {
    throw new Error(`Could not sample output frame at ${seconds}s: ${result.stderr?.toString() ?? ''}`)
  }
  return [result.stdout[0], result.stdout[1], result.stdout[2]]
}

function sampleTimeline(outputPath) {
  const samples = []
  for (let seconds = 0.5; seconds <= 6; seconds += 0.25) {
    try {
      samples.push({ seconds, rgb: sampleRgb(outputPath, seconds) })
    } catch {
      break
    }
  }
  return samples
}

function isRed(rgb) {
  return rgb[0] > 180 && rgb[1] < 80 && rgb[2] < 80
}

function isGreen(rgb) {
  return rgb[1] > 180 && rgb[0] < 80 && rgb[2] < 80
}

function verifyOutput(outputPath, statuses, sessionId) {
  if (!outputPath || !existsSync(outputPath)) {
    throw new Error(`Screens smoke output was not created: ${outputPath ?? 'missing path'}`)
  }
  const size = statSync(outputPath).size
  if (size <= 0) {
    throw new Error(`Screens smoke output is empty: ${outputPath}`)
  }

  const uniqueSessionIds = new Set(statuses.map((status) => status.sessionId).filter(Boolean))
  if (uniqueSessionIds.size !== 1 || !uniqueSessionIds.has(sessionId)) {
    throw new Error(`Screen switching appears to have restarted the session: ${[...uniqueSessionIds].join(', ')}`)
  }

  const samples = sampleTimeline(outputPath)
  const red = samples.find((sample) => isRed(sample.rgb))
  const green = red ? samples.find((sample) => sample.seconds > red.seconds && isGreen(sample.rgb)) : null
  const normal = green
    ? samples.find((sample) => sample.seconds > green.seconds && !isRed(sample.rgb) && !isGreen(sample.rgb))
    : null
  if (!red || !green || !normal) {
    const rendered = samples.map((sample) => `${sample.seconds.toFixed(2)}s=rgb(${sample.rgb.join(',')})`).join(', ')
    throw new Error(`Expected red -> green -> Normal Screen sequence, got ${rendered}`)
  }

  console.log(
    `Screens smoke OK - switched red -> green -> Normal in one session (${outputPath}, ${size} bytes).`
  )
}

async function assertSameRunningSession(ws, sessionId) {
  const status = await request(ws, timeoutMs, 'recording.status')
  if (status.sessionId !== sessionId || !['recording', 'streaming'].includes(status.state)) {
    throw new Error(`Expected same running session ${sessionId}, got ${status.sessionId}/${status.state}`)
  }
}

function sessionParams() {
  return {
    sources: { testPattern: true },
    layout: {
      layoutPreset: 'screen-only',
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
      video: { preset: 'custom', width: 640, height: 360, fps: 30, bitrateKbps: 2000 },
      rtmp: { preset: 'custom', serverUrl: '', streamKey: '' }
    }
  }
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
      rejectConnection(new Error(`Dev app exited before Screens smoke completed: code=${code} signal=${signal}`))
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

function sleep(ms) {
  return new Promise((resolveSleep) => setTimeout(resolveSleep, ms))
}
