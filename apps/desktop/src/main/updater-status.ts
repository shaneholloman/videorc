import type { UpdateStatus } from '../shared/backend'

// Pure mapping from electron-updater lifecycle events to the UpdateStatus the
// renderer consumes. Kept electron-free so it is unit-testable (the desktop test
// runner is node-only, no electron). updater.ts wires the real autoUpdater
// events to these helpers.

export type UpdaterEvent =
  | { type: 'checking' }
  | { type: 'available'; version: string }
  | { type: 'not-available'; currentVersion: string }
  | { type: 'progress'; percent: number }
  | { type: 'downloaded'; version: string }
  | { type: 'error'; message: string }
  | { type: 'unsupported' }

// electron-updater can report a percent slightly outside 0–100 (or NaN before the
// first chunk); normalise it so the progress bar never overflows.
export function clampPercent(percent: number): number {
  if (!Number.isFinite(percent)) {
    return 0
  }
  return Math.min(100, Math.max(0, Math.round(percent)))
}

export function updateStatusFromEvent(event: UpdaterEvent): UpdateStatus {
  switch (event.type) {
    case 'checking':
      return { phase: 'checking' }
    case 'available':
      return { phase: 'available', version: event.version }
    case 'not-available':
      return { phase: 'not-available', currentVersion: event.currentVersion }
    case 'progress':
      return { phase: 'downloading', percent: clampPercent(event.percent) }
    case 'downloaded':
      return { phase: 'downloaded', version: event.version }
    case 'error':
      return { phase: 'error', message: event.message }
    case 'unsupported':
      return { phase: 'unsupported' }
  }
}

// After a manual check resolves, start downloading immediately (one-click feel)
// only when an update is actually available.
export function shouldAutoDownload(status: UpdateStatus): boolean {
  return status.phase === 'available'
}
