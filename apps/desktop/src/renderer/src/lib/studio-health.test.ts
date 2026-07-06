import { describe, expect, it } from 'vitest'

import { studioHealth, type StudioHealthInput } from './studio-health'

function stats(overrides: Partial<StudioHealthInput> = {}): StudioHealthInput {
  return {
    compositorBackend: 'metal',
    compositorCpuFallbackFrames: 0,
    previewTransport: 'native-surface',
    previewSurfaceBacking: 'cametal-layer',
    ...overrides
  }
}

describe('studioHealth', () => {
  it('reports Live on a healthy Metal session while active', () => {
    expect(studioHealth(stats(), true)).toMatchObject({ tone: 'good', value: 'Live' })
  })

  it('reports Ready on a healthy Metal session while idle', () => {
    expect(studioHealth(stats(), false)).toMatchObject({ tone: 'good', value: 'Ready' })
  })

  it('degrades to "Preview may not match recording" on CPU fallback', () => {
    const result = studioHealth(stats({ compositorBackend: 'cpu-fallback' }), false)
    expect(result.tone).toBe('error')
    expect(result.value).toBe('Degraded')
    expect(result.detail).toContain('Preview may not match recording')
  })

  it('includes the fallback reason in the degraded detail when known', () => {
    const result = studioHealth(
      stats({ compositorBackend: 'cpu-fallback', compositorFallbackReason: 'Metal disabled' }),
      true
    )
    expect(result.detail).toBe('Preview may not match recording — Metal disabled')
  })

  it('degrades when CPU fallback frames appear mid-recording even if the backend label is metal', () => {
    expect(studioHealth(stats({ compositorCpuFallbackFrames: 5 }), true).tone).toBe('error')
  })

  it('does not degrade on stale CPU fallback frames while idle', () => {
    expect(studioHealth(stats({ compositorCpuFallbackFrames: 5 }), false)).toMatchObject({
      tone: 'good',
      value: 'Ready'
    })
  })

  it('warns when preview present latency exceeds the live budget', () => {
    expect(studioHealth(stats({ previewInputToPresentLatencyP95Ms: 120 }), true)).toMatchObject({
      tone: 'warn',
      value: 'Lagging'
    })
  })

  it('blocks production preview when image polling is the active transport', () => {
    expect(studioHealth(stats({ previewTransport: 'latest-jpeg-polling' }), true)).toMatchObject({
      tone: 'error',
      value: 'Blocked'
    })
  })

  it('blocks production preview when the Electron proof surface is the active transport', () => {
    expect(
      studioHealth(
        stats({
          previewTransport: 'electron-proof-surface',
          previewSurfaceBacking: 'electron-browser-window'
        }),
        false
      )
    ).toMatchObject({ tone: 'error', value: 'Blocked' })
  })

  it('blocks an active session when an OPEN preview has no native surface yet', () => {
    expect(
      studioHealth(
        stats({ previewTransport: 'unavailable', previewSurfaceBacking: 'none' }),
        true,
        {
          previewOpen: true
        }
      )
    ).toMatchObject({ tone: 'error', value: 'Blocked' })
  })

  // 0.9.10 by-eye: recording with the preview deliberately closed showed the
  // red "requires native CAMetalLayer … unavailable / none" banner. With no
  // preview open there is no preview path to police — the compositor checks
  // above own recording parity.
  it('stays healthy while recording with the preview closed (no transport, no problem)', () => {
    expect(
      studioHealth(
        stats({ previewTransport: 'unavailable', previewSurfaceBacking: 'none' }),
        true,
        {
          previewOpen: false
        }
      )
    ).toMatchObject({ tone: 'good', value: 'Live' })
  })

  it('still blocks a live non-native transport even with the preview window closed', () => {
    expect(
      studioHealth(stats({ previewTransport: 'latest-jpeg-polling' }), true, {
        previewOpen: false
      })
    ).toMatchObject({ tone: 'error', value: 'Blocked' })
  })

  it('allows debug fallback policy to warn when preview is on an image-polling transport', () => {
    expect(studioHealth(stats({ previewTransport: 'latest-jpeg-polling' }), true)).toMatchObject({
      tone: 'error',
      value: 'Blocked'
    })
    expect(
      studioHealth(stats({ previewTransport: 'latest-jpeg-polling' }), true, {
        requireNativePreview: false
      })
    ).toMatchObject({
      tone: 'warn',
      value: 'Fallback'
    })
  })

  it('keeps showing debug Fallback over Lagging when polling with high latency (no flapping)', () => {
    expect(
      studioHealth(
        stats({ previewTransport: 'latest-jpeg-polling', previewInputToPresentLatencyP95Ms: 200 }),
        true,
        { requireNativePreview: false }
      )
    ).toMatchObject({ tone: 'warn', value: 'Fallback' })
  })

  it('is neutral when no compositor has reported yet', () => {
    expect(studioHealth(stats({ compositorBackend: undefined }), false)).toMatchObject({
      tone: 'neutral',
      value: 'Idle'
    })
  })
})
