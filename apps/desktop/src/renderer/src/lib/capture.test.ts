import { describe, expect, it } from 'vitest'

import type { CaptureConfig } from './capture'
import {
  normalizeAudioSettings,
  normalizeMicrophoneSyncOffsetMs,
  normalizeVideoSettings,
  smokePreviewCompositorCaptureConfig,
  videoPresets
} from './capture'

describe('smokePreviewCompositorCaptureConfig', () => {
  it('uses a renderable test-pattern source instead of stopped real preview sources', () => {
    const config: Pick<CaptureConfig, 'sources' | 'layout' | 'video'> = {
      sources: {
        screenId: 'screen:1',
        screenName: 'Display',
        windowId: 'window:1',
        windowName: 'App',
        cameraId: 'camera:1',
        cameraName: 'Camera',
        microphoneId: 'microphone:1',
        microphoneName: 'Microphone'
      },
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
      video: {
        preset: 'tutorial-1440p30',
        width: 2560,
        height: 1440,
        fps: 30,
        bitrateKbps: 8000
      }
    }

    expect(smokePreviewCompositorCaptureConfig(config)).toEqual({
      ...config,
      sources: {
        microphoneId: 'microphone:1',
        microphoneName: 'Microphone',
        testPattern: true
      }
    })
  })
})

describe('normalizeMicrophoneSyncOffsetMs', () => {
  it('preserves exact measured millisecond offsets when the user has set one', () => {
    expect(normalizeMicrophoneSyncOffsetMs(-166)).toBe(-166)
    expect(
      normalizeAudioSettings({ microphoneSyncOffsetMs: -166, microphoneSyncOffsetUserSet: true })
        .microphoneSyncOffsetMs
    ).toBe(-166)
  })

  it('applies the calibrated default until the user sets an offset', () => {
    expect(normalizeAudioSettings({ microphoneSyncOffsetMs: 0 }).microphoneSyncOffsetMs).toBe(-350)
    expect(normalizeAudioSettings({}).microphoneSyncOffsetMs).toBe(-350)
  })

  it('clamps to the backend-supported range', () => {
    expect(normalizeMicrophoneSyncOffsetMs(-1200)).toBe(-1000)
    expect(normalizeMicrophoneSyncOffsetMs(1200)).toBe(1000)
  })

  it('uses the provided fallback when the value is not numeric', () => {
    expect(normalizeMicrophoneSyncOffsetMs('nope', -120)).toBe(-120)
  })
})

describe('videoPresets', () => {
  it('includes first-class 4K recording and platform-safe streaming presets', () => {
    expect(videoPresets['record-4k30']).toMatchObject({
      width: 3840,
      height: 2160,
      fps: 30,
      bitrateKbps: 30000
    })
    expect(videoPresets['record-4k60-experimental']).toMatchObject({
      width: 3840,
      height: 2160,
      fps: 60,
      bitrateKbps: 50000
    })
    expect(videoPresets['stream-safe-1080p30']).toMatchObject({
      width: 1920,
      height: 1080,
      fps: 30,
      bitrateKbps: 6000
    })
    expect(videoPresets['stream-safe-1080p60']).toMatchObject({
      width: 1920,
      height: 1080,
      fps: 60,
      bitrateKbps: 6000
    })
  })

  it('normalizes persisted first-class presets through the shared map', () => {
    expect(normalizeVideoSettings({ preset: 'record-4k30' })).toEqual(videoPresets['record-4k30'])
    expect(normalizeVideoSettings({ preset: 'stream-safe-1080p30' })).toEqual(
      videoPresets['stream-safe-1080p30']
    )
  })
})
