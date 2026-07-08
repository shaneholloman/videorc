import assert from 'node:assert/strict'
import { test } from 'node:test'

import {
  REQUIRED_WINDOWS_FFMPEG_ENCODERS,
  REQUIRED_WINDOWS_FFMPEG_PROTOCOLS,
  assessWindowsFfmpegCapabilities
} from './windows-ffmpeg-capabilities.mjs'

const PROTOCOLS_WITH_TLS = [
  'Supported file protocols:',
  'Input:',
  '  file',
  '  rtmp',
  '  rtmps',
  '  tls',
  'Output:',
  '  file',
  '  rtmp',
  '  rtmps',
  '  tls'
].join('\n')

const ENCODERS_WITH_MF = [
  'Encoders:',
  ' V....D h264_mf              MediaFoundation H.264 encoder (codec h264)',
  ' A....D aac                  AAC (Advanced Audio Coding)'
].join('\n')

test('a fully capable ffmpeg passes', () => {
  const result = assessWindowsFfmpegCapabilities({
    protocolsOutput: PROTOCOLS_WITH_TLS,
    encodersOutput: ENCODERS_WITH_MF
  })
  assert.equal(result.ok, true)
  assert.deepEqual(result.missing, [])
})

test('an ffmpeg without a TLS stack fails on rtmps and tls (the 0.9.23 class)', () => {
  const result = assessWindowsFfmpegCapabilities({
    protocolsOutput: PROTOCOLS_WITH_TLS.split('\n')
      .filter((line) => !/rtmps|tls/.test(line))
      .join('\n'),
    encodersOutput: ENCODERS_WITH_MF
  })
  assert.equal(result.ok, false)
  assert.deepEqual(result.missing, ['protocol:rtmps', 'protocol:tls'])
})

test('rtmps does not substring-match as rtmp', () => {
  const result = assessWindowsFfmpegCapabilities({
    protocolsOutput: 'Input:\n  rtmps\n  tls',
    encodersOutput: ENCODERS_WITH_MF
  })
  assert.equal(result.ok, false)
  assert.deepEqual(result.missing, ['protocol:rtmp'])
})

test('a missing MediaFoundation encoder fails', () => {
  const result = assessWindowsFfmpegCapabilities({
    protocolsOutput: PROTOCOLS_WITH_TLS,
    encodersOutput: 'Encoders:\n A....D aac    AAC'
  })
  assert.equal(result.ok, false)
  assert.deepEqual(result.missing, ['encoder:h264_mf'])
})

test('empty output reports the whole required set (fail closed)', () => {
  const result = assessWindowsFfmpegCapabilities({})
  assert.equal(result.ok, false)
  assert.deepEqual(result.missing, [
    ...REQUIRED_WINDOWS_FFMPEG_PROTOCOLS.map((name) => `protocol:${name}`),
    ...REQUIRED_WINDOWS_FFMPEG_ENCODERS.map((name) => `encoder:${name}`)
  ])
})
