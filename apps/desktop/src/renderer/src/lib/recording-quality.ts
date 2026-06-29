import type { FileAssessment, GateStatus } from '@/lib/backend'

export type RecordingQualityBadgeVariant = 'success' | 'warning' | 'destructive'
export type RecordingQualityAlertVariant = 'warning' | 'destructive'
export type RecordingQualitySource = 'automatic' | 'manual-check' | 'manual-repair'

export type RecordingQualityState = {
  label: string
  badgeVariant: RecordingQualityBadgeVariant
  reasons: string[]
  alertTitle: string | null
  alertVariant: RecordingQualityAlertVariant | null
  source: RecordingQualitySource
}

export function recordingQualityState({
  qualityStatus,
  assessment,
  result
}: {
  qualityStatus: GateStatus | null | undefined
  assessment: FileAssessment | null
  result: GateStatus | null
}): RecordingQualityState | null {
  if (result) {
    return stateFromGateStatus(result, 'manual-repair')
  }
  if (assessment) {
    return stateFromAssessment(assessment)
  }
  if (qualityStatus) {
    return stateFromGateStatus(qualityStatus, 'automatic')
  }
  return null
}

function stateFromGateStatus(
  status: GateStatus,
  source: Extract<RecordingQualitySource, 'automatic' | 'manual-repair'>
): RecordingQualityState {
  switch (status.status) {
    case 'ready':
      return cleanState(source)
    case 'repaired':
      return {
        label: status.interpolated ? 'Repaired - interpolated' : 'Repaired',
        badgeVariant: 'success',
        reasons: [],
        alertTitle: null,
        alertVariant: null,
        source
      }
    case 'not-hundred-percent':
      return {
        label: 'Not 100%',
        badgeVariant: 'warning',
        reasons: status.reasons,
        alertTitle: 'Why this is not 100%',
        alertVariant: 'warning',
        source
      }
    case 'failed':
      return {
        label: 'Check failed',
        badgeVariant: 'destructive',
        reasons: [status.reason],
        alertTitle: 'The quality check could not run',
        alertVariant: 'destructive',
        source
      }
  }
}

function stateFromAssessment(assessment: FileAssessment): RecordingQualityState {
  if (assessment.verdict === 'clean') {
    return cleanState('manual-check')
  }
  if (assessment.verdict === 'repairable') {
    return {
      label: 'Needs repair',
      badgeVariant: 'warning',
      reasons: assessment.reasons,
      alertTitle: 'Why this is not 100%',
      alertVariant: 'warning',
      source: 'manual-check'
    }
  }
  return {
    label: 'Not 100%',
    badgeVariant: 'destructive',
    reasons: assessment.reasons,
    alertTitle: 'Why this is not 100%',
    alertVariant: 'warning',
    source: 'manual-check'
  }
}

function cleanState(source: RecordingQualitySource): RecordingQualityState {
  return {
    label: '100%',
    badgeVariant: 'success',
    reasons: [],
    alertTitle: null,
    alertVariant: null,
    source
  }
}
