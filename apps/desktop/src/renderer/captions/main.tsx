import React, { useEffect, useState, type ReactElement } from 'react'
import ReactDOM from 'react-dom/client'

import { CaptionsReader } from '@/components/captions-reader'
import { AppErrorBoundary } from '@/components/error-boundary'
import type { CaptionsUpdate } from '@/lib/backend'
import '@/styles.css'

// Long-lived second window: drop React's dev perf-track measures, which buffer
// outside the V8 heap and leak over time (see videorc-react-dev-perf-track-leak).
if (import.meta.env.DEV && localStorage.getItem('videorc.reactPerfTrack') !== '1') {
  const nativeMeasure = performance.measure.bind(performance)
  performance.measure = (
    name: string,
    startOrOptions?: string | PerformanceMeasureOptions,
    endMark?: string
  ): PerformanceMeasure => {
    const detail =
      typeof startOrOptions === 'object' && startOrOptions !== null ? startOrOptions.detail : null
    if (detail && typeof detail === 'object' && 'devtools' in detail) {
      return undefined as unknown as PerformanceMeasure
    }
    return nativeMeasure(name, startOrOptions, endMark)
  }
}

// The window's data comes from the main renderer through the main-process
// relay: seed from the cached caption lines, then follow live pushes.
function CaptionsWindowApp(): ReactElement {
  const [lines, setLines] = useState<CaptionsUpdate[]>([])
  const [alwaysOnTop, setAlwaysOnTop] = useState(false)
  useEffect(() => {
    void window.videorc
      ?.getCaptionLines?.()
      .then((initial) => initial && setLines(initial))
      .catch(() => {})
    void window.videorc
      ?.getCaptionsWindowState?.()
      .then((state) => state && setAlwaysOnTop(state.alwaysOnTop))
      .catch(() => {})
    const offLines = window.videorc?.onCaptionLines?.((next) => setLines(next))
    const offState = window.videorc?.onCaptionsWindowState?.((state) =>
      setAlwaysOnTop(state.alwaysOnTop)
    )
    return () => {
      offLines?.()
      offState?.()
    }
  }, [])
  return (
    <CaptionsReader
      lines={lines}
      alwaysOnTop={alwaysOnTop}
      onToggleAlwaysOnTop={() => void window.videorc?.setCaptionsWindowAlwaysOnTop?.(!alwaysOnTop)}
    />
  )
}

ReactDOM.createRoot(document.getElementById('root') as HTMLElement).render(
  <React.StrictMode>
    <AppErrorBoundary>
      <CaptionsWindowApp />
    </AppErrorBoundary>
  </React.StrictMode>
)
