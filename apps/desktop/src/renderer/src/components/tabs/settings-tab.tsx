import { ArrowClockwise, CheckCircle, Database, GearSix, LockKey, Warning } from '@phosphor-icons/react'
import { useTheme } from 'next-themes'
import type { ReactElement } from 'react'

import { PanelSection } from '@/components/panel-section'
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
import { ToggleGroup, ToggleGroupItem } from '@/components/ui/toggle-group'
import { useStudio } from '@/hooks/use-studio'
import type { RtmpPreset, SystemPermissionPane, VideoPreset } from '@/lib/backend'

export function SettingsTab({ onResetOnboarding }: { onResetOnboarding: () => void }): ReactElement {
  const {
    settings,
    setSettings,
    health,
    captureConfig,
    applyVideoPreset,
    applyRtmpPreset,
    openSystemPermission
  } = useStudio()
  const { theme, setTheme } = useTheme()

  return (
    <div className="grid gap-4 lg:grid-cols-2">
      <PanelSection description="Where recordings are written and which FFmpeg binary is used." icon={GearSix} title="Storage & tools">
        <FieldGroup>
          <Field>
            <FieldLabel htmlFor="output-directory">Output directory</FieldLabel>
            <Input
              id="output-directory"
              placeholder="~/Movies/Videorc/Recordings"
              value={settings.outputDirectory}
              onChange={(event) => setSettings((current) => ({ ...current, outputDirectory: event.target.value }))}
            />
          </Field>
          <Field>
            <FieldLabel htmlFor="ffmpeg-path">FFmpeg path</FieldLabel>
            <Input
              id="ffmpeg-path"
              placeholder="ffmpeg"
              value={settings.ffmpegPath}
              onChange={(event) => setSettings((current) => ({ ...current, ffmpegPath: event.target.value }))}
            />
            <FieldDescription className="flex items-center gap-1.5">
              {health?.ffmpeg.available ? (
                <CheckCircle className="size-3.5 text-success" weight="fill" />
              ) : (
                <Warning className="size-3.5 text-warning" weight="fill" />
              )}
              {health?.ffmpeg.version ?? health?.ffmpeg.message ?? 'Waiting for FFmpeg status.'}
            </FieldDescription>
          </Field>
        </FieldGroup>
        <div className="flex items-center gap-2 rounded-lg border bg-muted/40 px-3 py-2 text-xs text-muted-foreground">
          <Database className="size-4 shrink-0" weight="duotone" />
          <span className="truncate">{health?.databasePath ?? 'Waiting for SQLite path.'}</span>
        </div>
      </PanelSection>

      <PanelSection description="Defaults applied to new capture sessions." icon={GearSix} title="Defaults">
        <FieldGroup>
          <Field>
            <FieldLabel htmlFor="default-preset">Default recording preset</FieldLabel>
            <Select value={captureConfig.video.preset} onValueChange={(value) => applyVideoPreset(value as VideoPreset)}>
              <SelectTrigger className="w-full" id="default-preset">
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
          </Field>
          <Field>
            <FieldLabel htmlFor="default-rtmp">Default RTMP preset</FieldLabel>
            <Select value={captureConfig.rtmpPreset} onValueChange={(value) => applyRtmpPreset(value as RtmpPreset)}>
              <SelectTrigger className="w-full" id="default-rtmp">
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
            <FieldLabel>Theme</FieldLabel>
            <ToggleGroup
              type="single"
              value={theme ?? 'system'}
              variant="outline"
              onValueChange={(value) => value && setTheme(value)}
            >
              <ToggleGroupItem value="light">Light</ToggleGroupItem>
              <ToggleGroupItem value="dark">Dark</ToggleGroupItem>
              <ToggleGroupItem value="system">System</ToggleGroupItem>
            </ToggleGroup>
          </Field>
        </FieldGroup>
        <div>
          <Button size="sm" variant="outline" onClick={onResetOnboarding}>
            <ArrowClockwise data-icon="inline-start" />
            Replay onboarding
          </Button>
        </div>
      </PanelSection>

      <PanelSection
        className="lg:col-span-2"
        description="Open the macOS panes used by screen, camera, and microphone capture."
        icon={LockKey}
        title="Permissions"
      >
        <div className="flex flex-wrap gap-2">
          {PERMISSION_SHORTCUTS.map((shortcut) => (
            <Button
              key={shortcut.pane}
              size="sm"
              variant="outline"
              onClick={() => void openSystemPermission(shortcut.pane)}
            >
              {shortcut.label}
            </Button>
          ))}
        </div>
      </PanelSection>
    </div>
  )
}

const PERMISSION_SHORTCUTS: Array<{ label: string; pane: SystemPermissionPane }> = [
  { label: 'Privacy', pane: 'privacy' },
  { label: 'Screen Recording', pane: 'screen-recording' },
  { label: 'Camera', pane: 'camera' },
  { label: 'Microphone', pane: 'microphone' }
]
