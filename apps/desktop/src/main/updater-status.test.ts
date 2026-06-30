import { describe, expect, it } from 'vitest'

import { clampPercent, shouldAutoDownload, updateStatusFromEvent } from './updater-status'

describe('updater status mapping', () => {
  it('maps each lifecycle event to its matching status', () => {
    expect(updateStatusFromEvent({ type: 'checking' })).toEqual({ phase: 'checking' })
    expect(updateStatusFromEvent({ type: 'available', version: '1.2.3' })).toEqual({
      phase: 'available',
      version: '1.2.3'
    })
    expect(updateStatusFromEvent({ type: 'not-available', currentVersion: '1.0.0' })).toEqual({
      phase: 'not-available',
      currentVersion: '1.0.0'
    })
    expect(updateStatusFromEvent({ type: 'downloaded', version: '1.2.3' })).toEqual({
      phase: 'downloaded',
      version: '1.2.3'
    })
    expect(updateStatusFromEvent({ type: 'error', message: 'nope' })).toEqual({
      phase: 'error',
      message: 'nope'
    })
    expect(updateStatusFromEvent({ type: 'unsupported' })).toEqual({ phase: 'unsupported' })
  })

  it('clamps and rounds download progress into 0–100', () => {
    expect(updateStatusFromEvent({ type: 'progress', percent: 42.7 })).toEqual({
      phase: 'downloading',
      percent: 43
    })
    expect(clampPercent(-5)).toBe(0)
    expect(clampPercent(150)).toBe(100)
    expect(clampPercent(Number.NaN)).toBe(0)
  })

  it('auto-downloads only when an update is available', () => {
    expect(shouldAutoDownload({ phase: 'available', version: '1.2.3' })).toBe(true)
    expect(shouldAutoDownload({ phase: 'not-available', currentVersion: '1.0.0' })).toBe(false)
    expect(shouldAutoDownload({ phase: 'idle' })).toBe(false)
    expect(shouldAutoDownload({ phase: 'downloaded', version: '1.2.3' })).toBe(false)
  })
})
