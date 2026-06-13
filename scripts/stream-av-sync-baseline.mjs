#!/usr/bin/env node
// Stream A/V sync baseline — plan WS-A slice A1 (Studio Shell And Live Control Plan).
//
// The record-only av-sync gate proves the local file; this proves what a platform
// RECEIVES. It runs two real-source sessions against the flash+click stimulus:
//
//   1. record-only          — the pre-encoded VideoToolbox MPEG-TS product path
//   2. record+stream        — the DEFAULT stream path (VideoToolbox H.264 -> FFmpeg copy -> tee),
//                             streaming to a local RTMP sink this harness owns
//                             (`ffmpeg -listen 1`), which records the received FLV
//
// then measures the flash/click A/V offset on all three outputs (record-only MKV,
// record+stream MKV leg, RTMP-received FLV), fits drift, classifies the plan's
// hypotheses (H1 timeline-start / H2 drift / H3 tee-leg divergence), and writes a
// JSON evidence file. Pass `--gate` to make the exit code reflect the verdict.
//
// REQUIREMENTS: same as the real-source baseline — a real desktop session with
// Screen Recording and Microphone permissions, speakers audible to the mic. The
// camera is irrelevant to the flash/click loop (screen flash + speaker click -> mic);
// pass VIDEORC_BASELINE_NO_CAMERA=1 when the dev app lacks camera permission.
//
//   node scripts/stream-av-sync-baseline.mjs [--gate] [--skip-record-only]
//
// Env:
//   VIDEORC_BASELINE_RECORDING_MS    per-session length (default 60000; >=600000 makes the drift gate binding)
//   VIDEORC_BASELINE_STREAM_PORT     local RTMP sink port (default 19501)
//   VIDEORC_SMOKE_OUTPUT_DIR         where session dirs + evidence land
//   VIDEORC_SMOKE_FFMPEG_PATH        ffmpeg for the sink and the measurements
//   (all other VIDEORC_BASELINE_* vars pass through to the underlying sessions)
//
// The stream session deliberately UNSETS VIDEORC_ENCODER_BRIDGE_VIDEO_OUTPUT so the
// backend's default selector chooses the product stream path. The stream key is a
// local dummy; no secrets are involved, and server URLs are still logged redacted.

import { spawn } from 'node:child_process'
import { existsSync, mkdirSync, readFileSync, statSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join, resolve } from 'node:path'

import { measureAvSync } from './lib/av-sync.mjs'
import {
  DEFAULT_STREAM_AV_SYNC_GATES,
  evaluateStreamAvSync,
  fitOffsetDrift,
  summarizeStreamAvSyncEvidence
} from './lib/stream-av-sync.mjs'

const argv = process.argv.slice(2).filter((arg) => arg !== '--')
const config = {
  gate: argv.includes('--gate'),
  skipRecordOnly: argv.includes('--skip-record-only'),
  recordingMs: Number(process.env.VIDEORC_BASELINE_RECORDING_MS ?? 60000),
  streamPort: Number(process.env.VIDEORC_BASELINE_STREAM_PORT ?? 19501),
  ffmpegPath: process.env.VIDEORC_SMOKE_FFMPEG_PATH ?? 'ffmpeg',
  sinkConnectGraceMs: 1500,
  sinkDrainTimeoutMs: 30000,
  outputRoot: resolve(
    process.env.VIDEORC_SMOKE_OUTPUT_DIR ?? join(tmpdir(), `videorc-stream-av-sync-${Date.now()}`)
  )
}
const STREAM_KEY = 'avsync-baseline' // local dummy key, not a secret
const serverUrl = `rtmp://127.0.0.1:${config.streamPort}/live`
const redactedStreamUrl = `${serverUrl}/••••`

let sink = null
let exitCode = 0
try {
  exitCode = await main()
} catch (error) {
  console.error(`stream av-sync baseline failed: ${error?.message ?? error}`)
  exitCode = 2
} finally {
  if (sink && sink.exitCode === null) sink.kill('SIGKILL')
}
process.exit(exitCode)

async function main() {
  mkdirSync(config.outputRoot, { recursive: true })
  const recordOnlyDir = join(config.outputRoot, 'record-only')
  const recordStreamDir = join(config.outputRoot, 'record-stream')
  const receivedFlvPath = join(recordStreamDir, 'stream-received.flv')
  mkdirSync(recordStreamDir, { recursive: true })

  // --- Session 1: record-only baseline (pre-encoded product path) ---------------
  let recordOnlyRecording = null
  let recordOnlyEvidence = null
  if (config.skipRecordOnly) {
    console.log('Skipping record-only baseline session (--skip-record-only).')
  } else {
    console.log(
      `\n=== Session 1/2: record-only av-sync baseline (${config.recordingMs / 1000}s) ===`
    )
    await runBaselineSession({
      outputDir: recordOnlyDir,
      env: {
        VIDEORC_ENCODER_BRIDGE_VIDEO_OUTPUT:
          process.env.VIDEORC_ENCODER_BRIDGE_VIDEO_OUTPUT ?? 'videotoolbox-h264-mpegts'
      }
    })
    recordOnlyEvidence = evidenceFromManifest(recordOnlyDir, 'record-only')
    recordOnlyRecording = recordingFromEvidence(recordOnlyEvidence, 'record-only')
  }

  // --- Session 2: record+stream against the local RTMP sink ---------------------
  console.log(
    `\n=== Session 2/2: record+stream av-sync against local RTMP sink (${config.recordingMs / 1000}s) ===`
  )
  if (process.env.VIDEORC_ENCODER_BRIDGE_VIDEO_OUTPUT) {
    console.log(
      `NOTE: ignoring VIDEORC_ENCODER_BRIDGE_VIDEO_OUTPUT=${process.env.VIDEORC_ENCODER_BRIDGE_VIDEO_OUTPUT} ` +
        'for the stream session — this gate exists to prove the DEFAULT stream selector path.'
    )
  }
  sink = spawnRtmpSink(receivedFlvPath)
  await sleep(config.sinkConnectGraceMs)
  if (sink.exitCode !== null) {
    throw new Error(
      `local RTMP sink exited before the session started (code ${sink.exitCode}) — is port ${config.streamPort} free?`
    )
  }
  console.log(`Local RTMP sink listening on ${redactedStreamUrl} -> ${receivedFlvPath}`)

  await runBaselineSession({
    outputDir: recordStreamDir,
    env: {
      VIDEORC_ENCODER_BRIDGE_VIDEO_OUTPUT: null, // backend default selector decides
      VIDEORC_BASELINE_STREAM: '1',
      VIDEORC_BASELINE_STREAM_SERVER_URL: serverUrl,
      VIDEORC_BASELINE_STREAM_KEY: STREAM_KEY
    }
  })
  const recordStreamEvidence = evidenceFromManifest(recordStreamDir, 'record+stream')
  const recordStreamRecording = recordingFromEvidence(recordStreamEvidence, 'record+stream')
  await drainSink()
  const receivedFlv =
    existsSync(receivedFlvPath) && statSync(receivedFlvPath).size > 0 ? receivedFlvPath : null
  if (!receivedFlv) {
    console.error('RTMP sink produced no received FLV — the stream leg never delivered data.')
  } else {
    console.log(
      `Received FLV: ${receivedFlv} (${(statSync(receivedFlv).size / (1024 * 1024)).toFixed(1)} MiB)`
    )
  }

  // --- Measure all three outputs -------------------------------------------------
  console.log('\nMeasuring flash/click A/V offsets…')
  const currentMicrophoneSyncOffsetMs = Number(process.env.VIDEORC_BASELINE_MIC_SYNC_OFFSET_MS ?? 0)
  const measureOptions = { ffmpegPath: config.ffmpegPath, currentMicrophoneSyncOffsetMs }
  const recordOnly = config.skipRecordOnly
    ? undefined
    : await measureOrNull(recordOnlyRecording, measureOptions)
  const recordStreamMkv = await measureOrNull(recordStreamRecording, measureOptions)
  const recordStreamFlv = await measureOrNull(receivedFlv, measureOptions)
  const flvDrift = fitOffsetDrift(recordStreamFlv?.pairs ?? [])
  const mkvDrift = fitOffsetDrift(recordStreamMkv?.pairs ?? [])

  const verdict = evaluateStreamAvSync({
    recordOnly,
    recordStreamMkv,
    recordStreamFlv,
    flvDrift,
    mkvDrift,
    durationSec: config.recordingMs / 1000
  })
  const driftEvidence = summarizeStreamAvSyncEvidence({
    recordStreamMkv,
    recordStreamFlv,
    flvDrift,
    mkvDrift
  })

  const evidencePath = join(config.outputRoot, 'stream-av-sync-evidence.json')
  writeFileSync(
    evidencePath,
    `${JSON.stringify(
      {
        generatedAtIso: new Date().toISOString(),
        platform: process.platform,
        command: { argv, gate: config.gate },
        config: {
          recordingMs: config.recordingMs,
          streamPort: config.streamPort,
          streamServerUrlRedacted: redactedStreamUrl,
          skipRecordOnly: config.skipRecordOnly,
          gates: DEFAULT_STREAM_AV_SYNC_GATES
        },
        sessions: {
          recordOnly: config.skipRecordOnly
            ? null
            : {
                directory: recordOnlyDir,
                recording: recordOnlyRecording,
                mediaQualityMode: recordOnlyEvidence?.result?.mediaQualityMode ?? null,
                splitOutputProof: splitOutputProof(recordOnlyEvidence),
                measurement: summarize(recordOnly)
              },
          recordStream: {
            directory: recordStreamDir,
            recording: recordStreamRecording,
            receivedFlv,
            mediaQualityMode: recordStreamEvidence?.result?.mediaQualityMode ?? null,
            splitOutputProof: splitOutputProof(recordStreamEvidence),
            measurementMkv: summarize(recordStreamMkv),
            measurementFlv: summarize(recordStreamFlv),
            mkvDrift,
            flvDrift,
            driftEvidence
          }
        },
        driftEvidence,
        verdict
      },
      null,
      2
    )}\n`
  )

  printSummary({
    recordOnly,
    recordStreamMkv,
    recordStreamFlv,
    flvDrift,
    mkvDrift,
    driftEvidence,
    verdict,
    evidencePath
  })
  return config.gate && !verdict.pass ? 1 : 0
}

// --- Session runner ---------------------------------------------------------------

function runBaselineSession({ outputDir, env }) {
  mkdirSync(outputDir, { recursive: true })
  const childEnv = { ...process.env }
  delete childEnv.VIDEORC_ENCODER_BRIDGE_VIDEO_OUTPUT
  for (const [key, value] of Object.entries(env)) {
    if (value === null) continue
    childEnv[key] = value
  }
  childEnv.VIDEORC_SMOKE_OUTPUT_DIR = outputDir
  childEnv.VIDEORC_BASELINE_AV_SYNC_STIMULUS = '1'
  childEnv.VIDEORC_BASELINE_RECORDING_MS = String(config.recordingMs)

  return new Promise((resolveRun, rejectRun) => {
    const child = spawn(process.execPath, [join('scripts', 'real-source-baseline-app.mjs')], {
      env: childEnv,
      stdio: 'inherit'
    })
    child.on('error', rejectRun)
    child.on('close', (code) => {
      if (code === 0) {
        resolveRun()
      } else {
        rejectRun(new Error(`real-source baseline session exited with code ${code}`))
      }
    })
  })
}

function evidenceFromManifest(outputDir, label) {
  const manifestPath = join(outputDir, 'latest-real-source-evidence.json')
  if (!existsSync(manifestPath)) {
    throw new Error(`${label} session left no evidence manifest at ${manifestPath}`)
  }
  const manifest = JSON.parse(readFileSync(manifestPath, 'utf8'))
  if (manifest?.result?.blockedBeforeEncoding) {
    throw new Error(
      `${label} session was blocked before encoding: ${manifest.result.acceptanceFailures?.[0] ?? 'unknown'}`
    )
  }
  return manifest
}

function recordingFromEvidence(manifest, label) {
  const recording = manifest?.paths?.recording
  if (!recording || !existsSync(recording)) {
    throw new Error(`${label} session produced no recording`)
  }
  return recording
}

function splitOutputProof(manifest) {
  const diagnostics = manifest?.diagnostics ?? {}
  return {
    recordingOutput: diagnostics.recordingOutput ?? null,
    streamOutput: diagnostics.streamOutput ?? null,
    rawVideoCopiedFrames: diagnostics.encoderBridgeRawVideoCopiedFrames ?? 0,
    metalTargetCopiedFrames: diagnostics.encoderBridgeMetalTargetCopiedFrames ?? 0,
    zeroCopyFrames: diagnostics.encoderBridgeZeroCopyFrames ?? 0,
    videoToolboxOutputFrames: diagnostics.encoderBridgeVideoToolboxOutputFrames ?? 0,
    videoToolboxOutputBytes: diagnostics.encoderBridgeVideoToolboxOutputBytes ?? 0,
    activeVideoToolboxOutputEncoders:
      diagnostics.encoderBridgeActiveVideoToolboxOutputEncoders ?? 0,
    recordingVideoToolboxOutputFrames:
      diagnostics.encoderBridgeRecordingVideoToolboxOutputFrames ?? 0,
    recordingVideoToolboxOutputBytes:
      diagnostics.encoderBridgeRecordingVideoToolboxOutputBytes ?? 0,
    streamVideoToolboxOutputFrames: diagnostics.encoderBridgeStreamVideoToolboxOutputFrames ?? 0,
    streamVideoToolboxOutputBytes: diagnostics.encoderBridgeStreamVideoToolboxOutputBytes ?? 0,
    separateOutputEncodersActive: diagnostics.encoderBridgeSeparateOutputEncodersActive === true
  }
}

// --- RTMP sink --------------------------------------------------------------------

function spawnRtmpSink(receivedFlvPath) {
  const child = spawn(
    config.ffmpegPath,
    [
      '-hide_banner',
      '-loglevel',
      'warning',
      '-y',
      '-listen',
      '1',
      '-f',
      'flv',
      '-i',
      `${serverUrl}/${STREAM_KEY}`,
      '-c',
      'copy',
      '-f',
      'flv',
      receivedFlvPath
    ],
    { stdio: ['ignore', 'inherit', 'inherit'] }
  )
  child.on('error', (error) => {
    console.error(`RTMP sink failed to spawn: ${error?.message ?? error}`)
  })
  return child
}

async function drainSink() {
  if (!sink) return
  if (sink.exitCode !== null) return
  console.log('Waiting for the RTMP sink to drain and finalize the received FLV…')
  const exited = await waitForExit(sink, config.sinkDrainTimeoutMs)
  if (!exited) {
    console.log('RTMP sink did not exit after disconnect — terminating it.')
    sink.kill('SIGTERM')
    if (!(await waitForExit(sink, 5000))) sink.kill('SIGKILL')
  }
}

function waitForExit(child, timeoutMs) {
  return new Promise((resolveWait) => {
    if (child.exitCode !== null) {
      resolveWait(true)
      return
    }
    const timer = setTimeout(() => {
      child.off('close', onClose)
      resolveWait(false)
    }, timeoutMs)
    const onClose = () => {
      clearTimeout(timer)
      resolveWait(true)
    }
    child.once('close', onClose)
  })
}

// --- Measurement helpers ------------------------------------------------------------

async function measureOrNull(filePath, options) {
  if (!filePath) return null
  try {
    return await measureAvSync(filePath, options)
  } catch (error) {
    console.error(`measureAvSync failed for ${filePath}: ${error?.message ?? error}`)
    return null
  }
}

function summarize(measurement) {
  if (!measurement) return null
  const { pairs, ...rest } = measurement
  return { ...rest, pairCount: pairs?.length ?? 0 }
}

function printSummary({
  recordOnly,
  recordStreamMkv,
  recordStreamFlv,
  flvDrift,
  mkvDrift,
  driftEvidence,
  verdict,
  evidencePath
}) {
  console.log('\n=== Stream A/V sync summary ===')
  printMeasurement('record-only MKV       ', recordOnly)
  printMeasurement('record+stream MKV leg ', recordStreamMkv)
  printMeasurement('RTMP-received FLV     ', recordStreamFlv)
  printDrift('MKV leg drift         ', mkvDrift)
  printDrift('received FLV drift    ', flvDrift)
  console.log(`classification         ${driftEvidence.classification}`)
  for (const finding of verdict.hypotheses) console.log(`HYPOTHESIS: ${finding}`)
  for (const warning of verdict.warnings) console.log(`WARN: ${warning}`)
  for (const failure of verdict.failures) console.log(`FAIL: ${failure}`)
  console.log(`Evidence: ${evidencePath}`)
  console.log(
    verdict.pass
      ? 'PASS — stream A/V sync inside the plan gate.'
      : 'FAIL — stream A/V sync outside the plan gate.'
  )
}

function printMeasurement(label, measurement) {
  if (measurement === undefined) {
    console.log(`${label} skipped`)
    return
  }
  if (measurement === null) {
    console.log(`${label} NOT MEASURED`)
    return
  }
  const median = measurement.medianOffsetMs
  const medianText = Number.isFinite(median)
    ? `${median >= 0 ? '+' : ''}${median.toFixed(0)}ms median`
    : 'no pairs'
  console.log(
    `${label} ${medianText} (${measurement.flashCount} flashes, ${measurement.clickCount} clicks, ` +
      `maxAbs ${Number.isFinite(measurement.maxAbsOffsetMs) ? measurement.maxAbsOffsetMs.toFixed(0) : '—'}ms)`
  )
}

function printDrift(label, drift) {
  if (!drift) {
    console.log(`${label} insufficient pairs for a fit`)
    return
  }
  console.log(
    `${label} ${drift.slopeMsPerMinute >= 0 ? '+' : ''}${drift.slopeMsPerMinute.toFixed(2)}ms/min ` +
      `(${(drift.slopeMsPerMinute * 30).toFixed(1)}ms/30min over ${drift.samples} pairs, span ${drift.spanSec.toFixed(0)}s)`
  )
}

function sleep(ms) {
  return new Promise((resolveSleep) => setTimeout(resolveSleep, ms))
}
