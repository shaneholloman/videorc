import { describe, expect, it } from 'vitest'

import type { UpdateStatus } from '@/lib/backend'

import { isUpdateInstallable } from './update-ui'

describe('isUpdateInstallable', () => {
  it('allows installing a downloaded update when nothing is capturing', () => {
    expect(isUpdateInstallable({ phase: 'downloaded', version: '1.0.0' }, false)).toBe(true)
  })

  it('blocks install while a capture is active (never interrupt a recording)', () => {
    expect(isUpdateInstallable({ phase: 'downloaded', version: '1.0.0' }, true)).toBe(false)
  })

  it('is false in every non-downloaded phase', () => {
    const phases: UpdateStatus[] = [
      { phase: 'idle' },
      { phase: 'checking' },
      { phase: 'available', version: '1.0.0' },
      { phase: 'downloading', percent: 50 },
      { phase: 'not-available', currentVersion: '1.0.0' },
      { phase: 'error', message: 'boom' },
      { phase: 'unsupported' }
    ]
    for (const status of phases) {
      expect(isUpdateInstallable(status, false)).toBe(false)
    }
  })
})
