import {
  Aperture,
  Broadcast,
  CaretRight,
  Clock,
  FrameCorners,
  Record,
  StopCircle,
  type Icon
} from '@phosphor-icons/react'
import type { ReactElement, ReactNode } from 'react'

import { PanelSection } from '@/components/panel-section'
import { StatusBadge } from '@/components/status-badge'
import { Button } from '@/components/ui/button'
import { Kbd } from '@/components/ui/kbd'
import { useWorkspaceNav } from '@/components/workspace-nav'
import { useStudio } from '@/hooks/use-studio'
import {
  outputSummary,
  recordingQuality,
  sessionMode,
  sessionStatusLabel,
  sessionStatusTone,
  streamingSummary
} from '@/lib/studio-session-view'

/**
 * Session panel (SD1): the glanceable session facts as a label→value list, plus
 * the Session Controls. Facts come straight from useStudio; the controls reuse
 * the transport handlers StudioTab owns (no second session state machine). The
 * Storage row is intentionally absent until F1 (disk-free space) lands — no
 * fake number. Navigable rows deep-link to the page that owns the setting.
 */
export function SessionPanel({
  active,
  recordingState,
  startRequestPending,
  recordBlockedReason,
  liveStreamBlockedReason,
  canStop,
  stopLabel,
  onRecord,
  onLiveStream,
  onStop
}: {
  active: boolean
  recordingState: string
  startRequestPending: boolean
  recordBlockedReason: string | null
  liveStreamBlockedReason: string | null
  canStop: boolean
  stopLabel: string
  onRecord: () => void
  onLiveStream: () => void
  onStop: () => void
}): ReactElement {
  const { captureConfig } = useStudio()
  const { openStudioPanel } = useWorkspaceNav()
  const video = captureConfig.video

  return (
    <PanelSection title="Session">
      <div className="flex flex-col gap-0.5">
        <SessionRow
          icon={Clock}
          label="Status"
          value={
            <StatusBadge
              tone={sessionStatusTone(recordingState)}
              value={sessionStatusLabel(recordingState)}
            />
          }
        />
        <SessionRow
          icon={Record}
          label="Mode"
          value={sessionMode(captureConfig.recordEnabled, captureConfig.streamEnabled)}
        />
        <SessionRow
          icon={Aperture}
          label="Output profile"
          value={recordingQuality(video)}
          onNavigate={() => openStudioPanel('recording')}
        />
        <SessionRow
          icon={Broadcast}
          label="Streaming"
          value={streamingSummary(captureConfig.streamEnabled, captureConfig.streaming.targets)}
          onNavigate={() => openStudioPanel('live')}
        />
        <SessionRow
          icon={FrameCorners}
          label="Output"
          value={outputSummary(video)}
          onNavigate={() => openStudioPanel('recording')}
        />
      </div>

      <div className="flex flex-col gap-2 rounded-row border bg-muted/20 p-3">
        <span className="text-xs font-medium text-muted-foreground">Session controls</span>
        <div className="flex gap-2">
          {active ? (
            <Button disabled={!canStop} size="sm" variant="destructive" onClick={onStop}>
              <StopCircle data-icon="inline-start" weight="fill" />
              {stopLabel}
            </Button>
          ) : (
            <>
              <Button
                disabled={Boolean(recordBlockedReason) || startRequestPending}
                size="sm"
                title={recordBlockedReason ?? undefined}
                variant="destructive"
                onClick={onRecord}
              >
                <Record data-icon="inline-start" weight="fill" />
                Start recording
                <Kbd className="ml-1.5">␣</Kbd>
              </Button>
              <Button
                disabled={Boolean(liveStreamBlockedReason) || startRequestPending}
                size="sm"
                title={liveStreamBlockedReason ?? undefined}
                variant="outline"
                onClick={onLiveStream}
              >
                <Broadcast data-icon="inline-start" weight="fill" />
                Stream
              </Button>
            </>
          )}
        </div>
      </div>
    </PanelSection>
  )
}

// Label→value row. Navigable rows render as a button with a trailing caret and
// deep-link to the owning page; static facts (Status, Mode) render as a div.
function SessionRow({
  icon: RowIcon,
  label,
  value,
  onNavigate
}: {
  icon: Icon
  label: string
  value: ReactNode
  onNavigate?: () => void
}): ReactElement {
  const body = (
    <>
      <RowIcon className="size-4 shrink-0 text-muted-foreground" weight="duotone" />
      <span className="flex-1 truncate text-left text-muted-foreground">{label}</span>
      <span className="flex min-w-0 items-center gap-1.5 font-medium text-foreground">
        {typeof value === 'string' ? <span className="truncate">{value}</span> : value}
        {onNavigate ? <CaretRight className="size-3.5 shrink-0 text-muted-foreground" /> : null}
      </span>
    </>
  )

  if (onNavigate) {
    return (
      <button
        className="flex items-center gap-3 rounded-row px-2.5 py-2 text-sm transition-colors hover:bg-accent"
        type="button"
        onClick={onNavigate}
      >
        {body}
      </button>
    )
  }
  return <div className="flex items-center gap-3 rounded-row px-2.5 py-2 text-sm">{body}</div>
}
