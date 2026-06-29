import { describe, expect, it } from 'vitest'

import type { FileAssessment, GateStatus } from '@/lib/backend'

import { recordingQualityState } from './recording-quality'

describe('recordingQualityState', () => {
  it('shows the persisted automatic gate before any manual check', () => {
    const qualityStatus: GateStatus = {
      status: 'not-hundred-percent',
      path: '/recordings/session.mp4',
      reasons: ['Frozen video segment detected.']
    }

    expect(recordingQualityState({ qualityStatus, assessment: null, result: null })).toMatchObject({
      label: 'Not 100%',
      badgeVariant: 'warning',
      reasons: ['Frozen video segment detected.'],
      alertTitle: 'Why this is not 100%',
      source: 'automatic'
    })
  })

  it('maps automatic pass and repaired statuses to success labels', () => {
    expect(
      recordingQualityState({
        qualityStatus: { status: 'ready', path: '/recordings/session.mp4' },
        assessment: null,
        result: null
      })
    ).toMatchObject({ label: '100%', badgeVariant: 'success', source: 'automatic' })

    expect(
      recordingQualityState({
        qualityStatus: {
          status: 'repaired',
          path: '/recordings/session.mp4',
          interpolated: true
        },
        assessment: null,
        result: null
      })
    ).toMatchObject({
      label: 'Repaired - interpolated',
      badgeVariant: 'success',
      source: 'automatic'
    })
  })

  it('lets manual assessment override the persisted automatic gate', () => {
    const assessment: FileAssessment = {
      path: '/recordings/session.mp4',
      verdict: 'repairable',
      issues: [{ kind: 'frozen-segments' }],
      reasons: ['Frozen video segment detected.'],
      repairable: true,
      hasBackup: false
    }

    expect(
      recordingQualityState({
        qualityStatus: { status: 'ready', path: '/recordings/session.mp4' },
        assessment,
        result: null
      })
    ).toMatchObject({
      label: 'Needs repair',
      badgeVariant: 'warning',
      source: 'manual-check'
    })
  })

  it('lets manual repair results override assessment and persisted state', () => {
    const result: GateStatus = {
      status: 'failed',
      path: '/recordings/session.mp4',
      reason: 'ffprobe could not read the file'
    }

    expect(
      recordingQualityState({
        qualityStatus: { status: 'ready', path: '/recordings/session.mp4' },
        assessment: {
          path: '/recordings/session.mp4',
          verdict: 'clean',
          issues: [],
          reasons: [],
          repairable: false,
          hasBackup: false
        },
        result
      })
    ).toMatchObject({
      label: 'Check failed',
      badgeVariant: 'destructive',
      reasons: ['ffprobe could not read the file'],
      alertTitle: 'The quality check could not run',
      source: 'manual-repair'
    })
  })
})
