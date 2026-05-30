import { existsSync, mkdirSync, statSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join, resolve } from 'node:path'
import { spawn } from 'node:child_process'

const repoRoot = resolve(import.meta.dirname, '..')
const appExecutable = resolve(
  repoRoot,
  process.env.VIDEOGRE_PACKAGED_APP_EXECUTABLE ??
    'apps/desktop/release/mac-arm64/Videogre.app/Contents/MacOS/Videogre'
)
const outputDirectory = resolve(
  process.env.VIDEOGRE_SMOKE_OUTPUT_DIR ??
    join(tmpdir(), `videogre-packaged-smoke-${Date.now()}`)
)
const ffmpegPath = process.env.VIDEOGRE_SMOKE_FFMPEG_PATH ?? 'ffmpeg'
const timeoutMs = Number(process.env.VIDEOGRE_SMOKE_TIMEOUT_MS ?? 45000)

if (process.platform !== 'darwin') {
  throw new Error('Packaged app smoke test currently targets macOS app bundles.')
}

if (!existsSync(appExecutable)) {
  throw new Error(`Packaged app executable not found: ${appExecutable}`)
}

mkdirSync(outputDirectory, { recursive: true })

let appProcess
let ws

try {
  const connection = await launchAndReadConnection()
  ws = await connectBackend(connection)
  const health = await request('health.ping', { ffmpegPath })
  if (!health?.ffmpeg?.available) {
    throw new Error(health?.ffmpeg?.message ?? 'FFmpeg is unavailable for packaged smoke recording.')
  }

  const started = await request('session.start', sessionParams())
  if (!['recording', 'streaming'].includes(started.state)) {
    throw new Error(`Expected recording state after start, got ${started.state}.`)
  }

  await sleep(1000)

  const stopped = await request('session.stop')
  const outputPath = stopped.outputPath ?? started.outputPath
  if (!outputPath || !existsSync(outputPath)) {
    throw new Error(`Recording output was not created: ${outputPath ?? 'missing path'}`)
  }

  const size = statSync(outputPath).size
  if (size <= 0) {
    throw new Error(`Recording output is empty: ${outputPath}`)
  }

  console.log(`Packaged smoke recording created: ${outputPath} (${size} bytes)`)
} finally {
  ws?.close()
  await stopApp()
}

function launchAndReadConnection() {
  return new Promise((resolveConnection, rejectConnection) => {
    const timer = setTimeout(() => {
      rejectConnection(new Error(`Timed out waiting for packaged backend READY after ${timeoutMs}ms.`))
    }, timeoutMs)

    appProcess = spawn(appExecutable, [], {
      env: {
        ...process.env,
        VIDEOGRE_SMOKE_PRINT_BACKEND_READY: '1'
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
      rejectConnection(new Error(`Packaged app exited before smoke test completed: code=${code} signal=${signal}`))
    })
  })
}

function handleAppOutput(text, resolveConnection, timer) {
  for (const line of text.split(/\r?\n/)) {
    if (line.trim()) {
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

function connectBackend(connection) {
  return new Promise((resolveConnection, rejectConnection) => {
    const url = `ws://${connection.host}:${connection.port}/ws?token=${encodeURIComponent(connection.token)}`
    ws = new WebSocket(url)
    ws.addEventListener('open', () => resolveConnection(ws), { once: true })
    ws.addEventListener('error', () => rejectConnection(new Error(`Could not connect to ${url}`)), {
      once: true
    })
  })
}

function request(method, params) {
  const id = `smoke-${Date.now()}-${Math.random().toString(16).slice(2)}`
  return new Promise((resolveRequest, rejectRequest) => {
    const timer = setTimeout(() => {
      rejectRequest(new Error(`Timed out waiting for ${method}.`))
    }, timeoutMs)

    const onMessage = (event) => {
      const message = JSON.parse(event.data)
      if (message.id !== id) {
        return
      }

      clearTimeout(timer)
      ws.removeEventListener('message', onMessage)
      if (message.ok) {
        resolveRequest(message.payload)
      } else {
        rejectRequest(new Error(message.error?.message ?? `${method} failed.`))
      }
    }

    ws.addEventListener('message', onMessage)
    ws.send(JSON.stringify({ id, method, params }))
  })
}

function sessionParams() {
  return {
    sources: {
      testPattern: true
    },
    layout: {
      cameraCorner: 'bottom-right',
      cameraSize: 'medium',
      cameraShape: 'rectangle',
      cameraMargin: 32,
      cameraFit: 'fill',
      cameraMirror: false,
      cameraZoom: 100,
      cameraOffsetX: 0,
      cameraOffsetY: 0
    },
    output: {
      recordEnabled: true,
      streamEnabled: false,
      outputDirectory,
      ffmpegPath,
      video: {
        preset: 'custom',
        width: 640,
        height: 360,
        fps: 30,
        bitrateKbps: 2000
      },
      rtmp: {
        preset: 'custom',
        serverUrl: '',
        streamKey: ''
      }
    }
  }
}

function sleep(ms) {
  return new Promise((resolveSleep) => setTimeout(resolveSleep, ms))
}

function stopApp() {
  return new Promise((resolveStop) => {
    if (!appProcess || appProcess.killed) {
      resolveStop()
      return
    }

    const timer = setTimeout(() => {
      appProcess.kill('SIGKILL')
      resolveStop()
    }, 3000)

    appProcess.once('exit', () => {
      clearTimeout(timer)
      resolveStop()
    })
    appProcess.kill('SIGTERM')
  })
}
