import { Broadcast, FileVideo } from '@phosphor-icons/react'
import type { ReactElement } from 'react'

import { PanelSection } from '@/components/panel-section'
import { Alert, AlertDescription, AlertTitle } from '@/components/ui/alert'
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import { Field, FieldContent, FieldDescription, FieldGroup, FieldLabel } from '@/components/ui/field'
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
import type { RtmpPreset, SessionSummary, VideoPreset } from '@/lib/backend'
import { dayLabel, durationMsLabel } from '@/lib/format'

export function OutputsTab(): ReactElement {
  const {
    captureConfig,
    setCaptureConfig,
    patchVideo,
    applyVideoPreset,
    applyRtmpPreset,
    streamReady,
    sessions,
    remuxSession
  } = useStudio()
  const { video } = captureConfig
  const outputSessions = sessions.filter((session) => session.outputPath || session.streamPreset)

  return (
    <div className="grid gap-4 lg:grid-cols-2">
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
        action={
          <Switch
            aria-label="Stream RTMP"
            checked={captureConfig.streamEnabled}
            onCheckedChange={(checked) => setCaptureConfig((current) => ({ ...current, streamEnabled: checked }))}
          />
        }
        description="Optional RTMP companion output. Stream keys are stored only here, never shown in Studio."
        icon={Broadcast}
        title="Streaming"
      >
        {captureConfig.streamEnabled && !streamReady ? (
          <Alert variant="warning">
            <Broadcast weight="fill" />
            <AlertTitle>Stream target incomplete</AlertTitle>
            <AlertDescription>An RTMP server and stream key are required before a session can start.</AlertDescription>
          </Alert>
        ) : null}

        <FieldGroup>
          <Field>
            <FieldLabel htmlFor="rtmp-preset">RTMP preset</FieldLabel>
            <Select value={captureConfig.rtmpPreset} onValueChange={(value) => applyRtmpPreset(value as RtmpPreset)}>
              <SelectTrigger className="w-full" id="rtmp-preset">
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                <SelectGroup>
                  <SelectItem value="youtube">YouTube</SelectItem>
                  <SelectItem value="twitch">Twitch</SelectItem>
                  <SelectItem value="x">X / Twitter</SelectItem>
                  <SelectItem value="custom">Custom RTMP</SelectItem>
                </SelectGroup>
              </SelectContent>
            </Select>
          </Field>
          <Field>
            <FieldLabel htmlFor="rtmp-server">RTMP server</FieldLabel>
            <Input
              id="rtmp-server"
              placeholder="rtmp://server/app"
              value={captureConfig.rtmpServerUrl}
              onChange={(event) =>
                setCaptureConfig((current) => ({ ...current, rtmpServerUrl: event.target.value }))
              }
            />
          </Field>
          <Field>
            <FieldLabel htmlFor="stream-key">Stream key</FieldLabel>
            <Input
              autoComplete="off"
              id="stream-key"
              placeholder="manual stream key"
              type="password"
              value={captureConfig.streamKey}
              onChange={(event) => setCaptureConfig((current) => ({ ...current, streamKey: event.target.value }))}
            />
            <FieldDescription>Kept locally and only sent to the RTMP server when streaming.</FieldDescription>
          </Field>
        </FieldGroup>

      </PanelSection>

      <PanelSection
        className="lg:col-span-2"
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

function OutputSessionRow({
  session,
  onRemux
}: {
  session: SessionSummary
  onRemux: () => void
}): ReactElement {
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
