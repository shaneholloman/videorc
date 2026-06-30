import type { UpdateStatus } from '@/lib/backend'

// Installing a downloaded update quits and relaunches Videorc, so it must never
// fire while a capture is live — never interrupt a recording. An update is
// installable only once it is fully downloaded AND nothing is recording or
// streaming.
export function isUpdateInstallable(status: UpdateStatus, captureActive: boolean): boolean {
  return status.phase === 'downloaded' && !captureActive
}
