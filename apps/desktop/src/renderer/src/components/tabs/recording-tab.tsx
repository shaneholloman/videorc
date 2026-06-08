import { FileVideo } from '@phosphor-icons/react'
import type { ReactElement } from 'react'

import { PanelSection } from '@/components/panel-section'
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import { Field, FieldDescription, FieldGroup, FieldLabel } from '@/components/ui/field'
import { Input } from '@/components/ui/input'
import {
  Select,
  SelectContent,
  SelectGroup,
  SelectItem,
  SelectTrigger,
  SelectValue
} from '@/components/ui/select'
import { Switch } from '@/components/ui/switch'
import { useStudio } from '@/hooks/use-studio'
import type { SessionSummary, VideoPreset } from '@/lib/backend'
import { dayLabel, durationMsLabel } from '@/lib/format'

export function RecordingTab(): ReactElement {
  const { captureConfig, setCaptureConfig, patchVideo, applyVideoPreset, sessions, remuxSession } = useStudio()
  const { video } = captureConfig
  const outputSessions = sessions.filter((session) => session.outputPath || session.streamPreset)

  return (
    <div className="grid gap-4">
      <PanelSection
        action={
          <Switch
            aria-label="Record MKV"
            checked={captureConfig.recordEnabled}
            onCheckedChange={(checked) => setCaptureConfig((current) => ({ ...current, recordEnabled: checked }))}
          />
        }
        description="Local recording exports MP4 into the recordings folder after capture finalizes."
        icon={FileVideo}
        title="Recording"
      >
        <FieldGroup>
          <Field>
            <FieldLabel htmlFor="video-preset">Video preset</FieldLabel>
            <Select value={video.preset} onValueChange={(value) => applyVideoPreset(value as VideoPreset)}>
              <SelectTrigger className="w-full" id="video-preset">
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                <SelectGroup>
                  <SelectItem value="record-4k30">Record 4K30</SelectItem>
                  <SelectItem value="record-4k60-experimental">Record 4K60 experimental</SelectItem>
                  <SelectItem value="stream-safe-1080p30">Stream-safe 1080p30</SelectItem>
                  <SelectItem value="stream-safe-1080p60">Stream-safe 1080p60</SelectItem>
                  <SelectItem value="tutorial-1440p30">Tutorial 1440p30</SelectItem>
                  <SelectItem value="tutorial-1080p30">Tutorial 1080p30</SelectItem>
                  <SelectItem value="stream-1080p60">Stream 1080p60</SelectItem>
                  <SelectItem value="custom">Custom</SelectItem>
                </SelectGroup>
              </SelectContent>
            </Select>
            <FieldDescription>Editing a value below switches the preset to Custom.</FieldDescription>
          </Field>

          <div className="grid grid-cols-2 gap-4">
            <NumberField
              label="Width"
              max={3840}
              min={640}
              value={video.width}
              onChange={(width) => patchVideo({ width })}
            />
            <NumberField
              label="Height"
              max={2160}
              min={360}
              value={video.height}
              onChange={(height) => patchVideo({ height })}
            />
            <NumberField label="FPS" max={60} min={24} value={video.fps} onChange={(fps) => patchVideo({ fps })} />
            <NumberField
              label="Bitrate kbps"
              max={50000}
              min={1000}
              step={500}
              value={video.bitrateKbps}
              onChange={(bitrateKbps) => patchVideo({ bitrateKbps })}
            />
          </div>
        </FieldGroup>
      </PanelSection>

      <PanelSection
        description="Completed local recording artifacts live here."
        icon={FileVideo}
        title="Recording artifacts"
      >
        {outputSessions.length ? (
          <div className="grid gap-2 lg:grid-cols-2">
            {outputSessions.map((session) => (
              <OutputSessionRow key={session.id} session={session} onRemux={() => remuxSession(session.id)} />
            ))}
          </div>
        ) : (
          <p className="text-sm text-muted-foreground">Completed local recordings will appear here.</p>
        )}
      </PanelSection>
    </div>
  )
}

function OutputSessionRow({ session, onRemux }: { session: SessionSummary; onRemux: () => void }): ReactElement {
  const canRemux = Boolean(session.status === 'completed' && session.outputPath?.endsWith('.mkv') && !session.mp4Path)

  return (
    <div className="flex items-center justify-between gap-3 rounded-lg border bg-muted/30 px-3 py-2">
      <div className="min-w-0">
        <div className="truncate text-sm font-semibold">{session.title}</div>
        <div className="truncate text-xs text-muted-foreground">
          {dayLabel(session.startedAt)} · {session.status} · {session.mp4Path ?? session.outputPath ?? session.streamPreset}
        </div>
        <div className="mt-1 flex flex-wrap gap-1.5">
          {session.container ? <Badge variant="outline">{session.container.toUpperCase()}</Badge> : null}
          {typeof session.durationMs === 'number' ? (
            <Badge variant="secondary">{durationMsLabel(session.durationMs)}</Badge>
          ) : null}
          {session.mp4Path ? <Badge variant="success">MP4</Badge> : <Badge variant="outline">MKV</Badge>}
        </div>
      </div>
      <Button disabled={!canRemux} size="sm" variant="outline" onClick={onRemux}>
        Export MP4
      </Button>
    </div>
  )
}

function NumberField({
  label,
  value,
  min,
  max,
  step,
  onChange
}: {
  label: string
  value: number
  min: number
  max: number
  step?: number
  onChange: (value: number) => void
}): ReactElement {
  return (
    <Field>
      <FieldLabel>{label}</FieldLabel>
      <Input
        max={max}
        min={min}
        step={step}
        type="number"
        value={value}
        onChange={(event) => onChange(Number(event.target.value))}
      />
    </Field>
  )
}
