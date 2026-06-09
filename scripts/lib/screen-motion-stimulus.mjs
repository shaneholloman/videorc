import { spawn, spawnSync } from 'node:child_process'
import { existsSync, mkdtempSync, rmSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { pathToFileURL } from 'node:url'

const DEFAULT_CHROME_PATH = '/Applications/Google Chrome.app/Contents/MacOS/Google Chrome'

export async function launchScreenMotionStimulus(options = {}) {
  const displayOptions = screenMotionStimulusOptionsForSource(options.screenSource) ?? {}
  const browserPath = options.browserPath ?? process.env.VIDEORC_SCREEN_MOTION_BROWSER_PATH ?? DEFAULT_CHROME_PATH
  const x = Number(options.x ?? process.env.VIDEORC_SCREEN_MOTION_X ?? displayOptions.x ?? 32)
  const y = Number(options.y ?? process.env.VIDEORC_SCREEN_MOTION_Y ?? displayOptions.y ?? 32)
  const width = Number(options.width ?? process.env.VIDEORC_SCREEN_MOTION_WIDTH ?? displayOptions.width ?? 1360)
  const height = Number(options.height ?? process.env.VIDEORC_SCREEN_MOTION_HEIGHT ?? displayOptions.height ?? 820)
  const settleMs = Number(options.settleMs ?? process.env.VIDEORC_SCREEN_MOTION_SETTLE_MS ?? 1800)

  if (!existsSync(browserPath)) {
    throw new Error(
      `Screen motion stimulus requires a Chromium-compatible browser. ` +
        `Set VIDEORC_SCREEN_MOTION_BROWSER_PATH, or install Google Chrome at ${browserPath}.`
    )
  }

  const dir = mkdtempSync(join(tmpdir(), 'videorc-screen-motion-'))
  const htmlPath = join(dir, 'stimulus.html')
  const profileDir = join(dir, 'profile')
  writeFileSync(htmlPath, stimulusHtml(), 'utf8')

  const child = spawn(
    browserPath,
    [
      `--user-data-dir=${profileDir}`,
      '--no-first-run',
      '--no-default-browser-check',
      '--disable-background-networking',
      '--disable-extensions',
      '--autoplay-policy=no-user-gesture-required',
      '--force-device-scale-factor=1',
      `--window-position=${x},${y}`,
      `--window-size=${width},${height}`,
      `--app=${pathToFileURL(htmlPath).href}`,
    ],
    {
      detached: true,
      stdio: 'ignore',
    }
  )
  child.unref()
  await sleep(settleMs)
  if (child.exitCode !== null) {
    rmSync(dir, { recursive: true, force: true })
    throw new Error(`Screen motion stimulus browser exited early with code ${child.exitCode}.`)
  }
  return { child, dir, htmlPath, browserPath, x, y, width, height }
}

export function screenMotionStimulusOptionsForSource(source) {
  const displayId = parseScreencaptureKitDisplayId(source?.id)
  if (!displayId || process.platform !== 'darwin') return null
  const bounds = queryMacDisplayBounds(displayId)
  return bounds ? stimulusWindowOptionsFromDisplayBounds(bounds) : null
}

export function stimulusWindowOptionsFromDisplayBounds(bounds, margin = 16) {
  if (!bounds || !Number.isFinite(bounds.width) || !Number.isFinite(bounds.height)) return null
  return {
    x: Math.round((bounds.x ?? 0) + margin),
    y: Math.round((bounds.y ?? 0) + margin),
    width: Math.max(640, Math.round(bounds.width - margin * 2)),
    height: Math.max(480, Math.round(bounds.height - margin * 2)),
  }
}

function parseScreencaptureKitDisplayId(id) {
  const match = String(id ?? '').match(/^screen:screencapturekit:(\d+)$/)
  return match ? Number(match[1]) : null
}

function queryMacDisplayBounds(displayId) {
  const result = spawnSync(
    'swift',
    [
      '-e',
      `import CoreGraphics
let id = CGDirectDisplayID(${displayId})
let bounds = CGDisplayBounds(id)
print("\\(bounds.origin.x),\\(bounds.origin.y),\\(bounds.width),\\(bounds.height)")`,
    ],
    { encoding: 'utf8', timeout: 5000 }
  )
  if (result.status !== 0) return null
  const values = result.stdout
    .trim()
    .split(',')
    .map((value) => Number(value))
  if (values.length !== 4 || values.some((value) => !Number.isFinite(value))) return null
  const [x, y, width, height] = values
  return { x, y, width, height }
}

export async function stopScreenMotionStimulus(stimulus) {
  if (!stimulus) return
  const pid = stimulus.child?.pid
  if (pid) {
    signal(pid, 'SIGTERM')
    await sleep(800)
    signal(pid, 'SIGKILL')
  }
  if (stimulus.dir) {
    rmSync(stimulus.dir, { recursive: true, force: true })
  }
}

function signal(pid, sig) {
  try {
    process.kill(-pid, sig)
  } catch {
    try {
      process.kill(pid, sig)
    } catch {
      // Already gone.
    }
  }
}

function sleep(ms) {
  return new Promise((resolve) => setTimeout(resolve, ms))
}

function stimulusHtml() {
  return `<!doctype html>
<html>
<head>
  <meta charset="utf-8">
  <title>Videorc Motion Stimulus</title>
  <style>
    html, body {
      margin: 0;
      width: 100%;
      height: 100%;
      overflow: hidden;
      background: #050505;
      color: white;
      font-family: ui-monospace, SFMono-Regular, Menlo, Consolas, monospace;
    }
    #stage {
      position: fixed;
      inset: 0;
      background:
        linear-gradient(90deg, #050505 0 11%, #ffffff 11% 12%, #050505 12% 23%, #00d5ff 23% 24%, #050505 24% 100%),
        repeating-linear-gradient(0deg, transparent 0 34px, rgba(255,255,255,0.18) 34px 36px);
    }
    .bar {
      position: absolute;
      top: 0;
      bottom: 0;
      width: 10vw;
      mix-blend-mode: screen;
      opacity: 0.86;
      will-change: transform;
    }
    #cyan { background: #00e5ff; }
    #magenta { background: #ff2bd6; }
    #yellow { background: #ffe84a; }
    #white { background: white; opacity: 0.72; }
    #ticker {
      position: absolute;
      left: 0;
      right: 0;
      bottom: 5vh;
      font-size: 4.2vh;
      white-space: nowrap;
      color: #050505;
      background: #f8f8f8;
      padding: 0.35em 0;
      will-change: transform;
    }
    #counter {
      position: absolute;
      left: 4vw;
      top: 5vh;
      font-size: 8vh;
      font-weight: 800;
      color: #ffffff;
      text-shadow: 0 0 8px #000, 0 0 2px #000;
    }
    #cursor {
      position: absolute;
      width: 8vh;
      height: 8vh;
      border: 1.2vh solid #fff;
      border-radius: 50%;
      box-shadow: 0 0 0 0.8vh #111, 0 0 0 1.4vh #00ff6a;
      will-change: transform;
    }
    #patches {
      position: absolute;
      right: 3vw;
      top: 4vh;
      display: grid;
      grid-template-columns: repeat(4, 7vw);
      grid-auto-rows: 7vw;
      gap: 0.6vw;
    }
    #patches div { border: 0.25vw solid #111; }
  </style>
</head>
<body>
  <div id="stage"></div>
  <div id="cyan" class="bar"></div>
  <div id="magenta" class="bar"></div>
  <div id="yellow" class="bar"></div>
  <div id="white" class="bar"></div>
  <div id="counter">frame 000000</div>
  <div id="cursor"></div>
  <div id="ticker">VIDEORC REAL-SCREEN MOTION STIMULUS - scrolling text, moving bars, cursor loop, color patches - OBS parity motion gate - </div>
  <div id="patches">
    <div style="background:#fff"></div><div style="background:#000"></div><div style="background:#ff2b2b"></div><div style="background:#31ff74"></div>
    <div style="background:#1d6fff"></div><div style="background:#ffe84a"></div><div style="background:#ff2bd6"></div><div style="background:#00e5ff"></div>
  </div>
  <script>
    const bars = [
      document.getElementById('cyan'),
      document.getElementById('magenta'),
      document.getElementById('yellow'),
      document.getElementById('white'),
    ];
    const counter = document.getElementById('counter');
    const cursor = document.getElementById('cursor');
    const ticker = document.getElementById('ticker');
    let frame = 0;
    function tick(now) {
      frame += 1;
      const w = window.innerWidth;
      const h = window.innerHeight;
      bars.forEach((bar, index) => {
        const phase = (now * (0.05 + index * 0.012) + index * 260) % (w + 240);
        bar.style.transform = 'translateX(' + (phase - 140) + 'px)';
      });
      const x = (Math.sin(now / 730) * 0.42 + 0.5) * (w - 120);
      const y = (Math.cos(now / 910) * 0.38 + 0.5) * (h - 120);
      cursor.style.transform = 'translate(' + x + 'px,' + y + 'px)';
      ticker.style.transform = 'translateX(' + (-((now / 9) % Math.max(1, w))) + 'px)';
      counter.textContent = 'frame ' + String(frame).padStart(6, '0');
      requestAnimationFrame(tick);
    }
    requestAnimationFrame(tick);
  </script>
</body>
</html>`
}
