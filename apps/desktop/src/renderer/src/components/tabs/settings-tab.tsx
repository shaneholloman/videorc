import {
  ArrowClockwise,
  Bug,
  CaretDown,
  CheckCircle,
  CircleNotch,
  Database,
  DownloadSimple,
  FolderOpen,
  GearSix,
  LockKey,
  Sparkle,
  Warning
} from '@phosphor-icons/react'
import { useTheme } from 'next-themes'
import type { ReactElement } from 'react'

import { ConfigGrid } from '@/components/page'
import { PanelSection } from '@/components/panel-section'
import { Alert, AlertDescription } from '@/components/ui/alert'
import { Button } from '@/components/ui/button'
import { Collapsible, CollapsibleContent, CollapsibleTrigger } from '@/components/ui/collapsible'
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
import { VideoPresetSelectItems } from '@/components/video-preset-select-items'
import { useStudio } from '@/hooks/use-studio'
import { useUpdater } from '@/hooks/use-updater'
import type { RtmpPreset, SystemPermissionPane, UpdateStatus, VideoPreset } from '@/lib/backend'
import { videoProfileEntitlementGate } from '@/lib/entitlement-ui'
import { isActiveRecordingState } from '@/lib/format'
import { VIDEORC_PREMIUM_URL } from '@/lib/premium-upgrade'
import { isUpdateInstallable } from '@/lib/update-ui'

export function SettingsTab({
  onResetOnboarding
}: {
  onResetOnboarding: () => void
}): ReactElement {
  const {
    settings,
    setSettings,
    health,
    captureConfig,
    applyVideoPreset,
    applyRtmpPreset,
    openSystemPermission,
    entitlements,
    exportSupportBundle,
    supportBundleExportPending
  } = useStudio()
  const { theme, setTheme } = useTheme()
  const defaultProfileGate = videoProfileEntitlementGate({
    entitlements,
    kind: 'recording',
    video: captureConfig.video
  })
  const defaultProfileEntitlementMessage = defaultProfileGate.allowed
    ? null
    : defaultProfileGate.reason

  const locateFfmpeg = async (): Promise<void> => {
    const path = await window.videorc?.pickFile?.()
    if (path) {
      setSettings((current) => ({ ...current, ffmpegPath: path }))
    }
  }

  return (
    <ConfigGrid>
      <PanelSection
        description="Where recordings are written and which FFmpeg binary is used."
        icon={GearSix}
        title="Storage & tools"
      >
        <FieldGroup>
          <Field>
            <FieldLabel htmlFor="output-directory">Output directory</FieldLabel>
            <Input
              id="output-directory"
              placeholder="~/Movies/Videorc/Recordings"
              value={settings.outputDirectory}
              onChange={(event) =>
                setSettings((current) => ({ ...current, outputDirectory: event.target.value }))
              }
            />
          </Field>
        </FieldGroup>

        {/* FFmpeg ships bundled with the packaged app, so normal users never set
            a path. Show a quiet status; surface a friendly, actionable card only
            when it is genuinely missing; keep the manual override in Advanced. */}
        {health?.ffmpeg.available ? (
          <div className="flex items-center gap-1.5 text-xs text-muted-foreground">
            <CheckCircle className="size-3.5 shrink-0 text-success" weight="fill" />
            <span className="truncate">
              FFmpeg ready{health.ffmpeg.version ? ` · ${health.ffmpeg.version}` : ''}
            </span>
          </div>
        ) : health ? (
          <div className="flex flex-col gap-2 rounded-row border border-warning/40 bg-warning/10 p-3">
            <div className="flex items-center gap-2 text-sm font-medium text-warning-foreground dark:text-warning">
              <Warning className="size-4 shrink-0" weight="fill" />
              Recording needs FFmpeg
            </div>
            <p className="text-xs text-muted-foreground">
              {import.meta.env.DEV
                ? 'For local development, install it with “brew install ffmpeg” — or locate an existing binary.'
                : 'FFmpeg ships with Videorc, so this usually means the install is damaged. Reinstall Videorc, or locate the binary manually.'}
            </p>
            <Button
              className="w-fit"
              size="sm"
              variant="outline"
              onClick={() => void locateFfmpeg()}
            >
              <FolderOpen data-icon="inline-start" />
              Locate FFmpeg…
            </Button>
          </div>
        ) : (
          <p className="text-xs text-muted-foreground">Checking for FFmpeg…</p>
        )}

        <Collapsible>
          <CollapsibleTrigger className="group flex w-fit items-center gap-2 rounded-row px-2 py-1.5 text-sm text-muted-foreground transition-colors hover:bg-accent hover:text-foreground">
            <CaretDown className="size-3.5 shrink-0 transition-transform group-data-[state=open]:rotate-180" />
            <span>Advanced</span>
          </CollapsibleTrigger>
          <CollapsibleContent className="pt-2">
            <Field>
              <FieldLabel htmlFor="ffmpeg-path">FFmpeg path override</FieldLabel>
              <Input
                id="ffmpeg-path"
                placeholder="ffmpeg (bundled)"
                value={settings.ffmpegPath}
                onChange={(event) =>
                  setSettings((current) => ({ ...current, ffmpegPath: event.target.value }))
                }
              />
              <FieldDescription>
                Leave blank to use the FFmpeg that ships with Videorc. Set a path only to point at a
                custom build.
              </FieldDescription>
            </Field>
          </CollapsibleContent>
        </Collapsible>
        <div className="flex items-center gap-2 rounded-row border bg-muted/40 px-3 py-2 text-xs text-muted-foreground">
          <Database className="size-4 shrink-0" weight="duotone" />
          <span className="truncate">{health?.databasePath ?? 'Waiting for SQLite path.'}</span>
        </div>
      </PanelSection>

      <PanelSection
        description="Defaults applied to new capture sessions."
        icon={GearSix}
        title="Defaults"
      >
        <FieldGroup>
          <Field>
            <FieldLabel htmlFor="default-preset">Default recording preset</FieldLabel>
            <Select
              value={captureConfig.video.preset}
              onValueChange={(value) =>
                applyVideoPreset(value as VideoPreset, { kind: 'recording' })
              }
            >
              <SelectTrigger className="w-full" id="default-preset">
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                <VideoPresetSelectItems entitlements={entitlements} kind="recording" />
              </SelectContent>
            </Select>
            {defaultProfileEntitlementMessage ? (
              <Alert variant="warning">
                <Warning className="size-4" weight="fill" />
                <AlertDescription className="flex flex-wrap items-center gap-2">
                  <span>{defaultProfileEntitlementMessage}</span>
                  {!defaultProfileGate.allowed && defaultProfileGate.upgradeUrl ? (
                    <Button
                      size="xs"
                      variant="outline"
                      onClick={() => openExternalUrl(VIDEORC_PREMIUM_URL)}
                    >
                      View Premium
                    </Button>
                  ) : null}
                </AlertDescription>
              </Alert>
            ) : null}
          </Field>
          <Field>
            <FieldLabel htmlFor="default-rtmp">Default RTMP preset</FieldLabel>
            <Select
              value={captureConfig.rtmpPreset}
              onValueChange={(value) => applyRtmpPreset(value as RtmpPreset)}
            >
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
        <div className="flex flex-col gap-2">
          <div className="flex flex-wrap gap-2">
            <Button size="sm" variant="outline" onClick={onResetOnboarding}>
              <ArrowClockwise data-icon="inline-start" />
              Replay onboarding
            </Button>
            <Button
              disabled={supportBundleExportPending}
              size="sm"
              variant="outline"
              onClick={() => void exportSupportBundle()}
            >
              <Bug data-icon="inline-start" />
              {supportBundleExportPending ? 'Exporting…' : 'Export support bundle'}
            </Button>
          </div>
          <p className="text-xs text-muted-foreground">
            Reporting a problem? Export a support bundle (redacted logs + diagnostics) to share with
            us.
          </p>
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

      <AboutAndUpdates />
    </ConfigGrid>
  )
}

function AboutAndUpdates(): ReactElement {
  const { runtimeInfo, recording } = useStudio()
  const { status, check, install } = useUpdater()
  const captureActive = isActiveRecordingState(recording.state)

  return (
    <PanelSection
      className="lg:col-span-2"
      description="Check for new versions of Videorc and install them."
      icon={Sparkle}
      title="About & updates"
    >
      <div className="flex flex-col gap-4">
        <div className="flex items-center justify-between gap-3">
          <span className="text-sm text-muted-foreground">Current version</span>
          <span className="font-mono text-sm text-foreground">{runtimeInfo?.version ?? '—'}</span>
        </div>
        <UpdateControl
          captureActive={captureActive}
          status={status}
          onCheck={check}
          onInstall={install}
        />
      </div>
    </PanelSection>
  )
}

function UpdateControl({
  status,
  captureActive,
  onCheck,
  onInstall
}: {
  status: UpdateStatus
  captureActive: boolean
  onCheck: () => void
  onInstall: () => void
}): ReactElement {
  switch (status.phase) {
    case 'unsupported':
      return (
        <p className="text-xs text-muted-foreground">
          Updates apply to the installed app — not to development builds.
        </p>
      )
    case 'checking':
      return (
        <Button disabled className="w-fit" size="sm" variant="outline">
          <CircleNotch className="animate-spin" data-icon="inline-start" />
          Checking for updates…
        </Button>
      )
    case 'available':
      return (
        <div className="flex items-center gap-2 text-sm text-muted-foreground">
          <DownloadSimple className="size-4 shrink-0" />
          <span>Version {status.version} available — starting download…</span>
        </div>
      )
    case 'downloading':
      return (
        <div className="flex flex-col gap-2">
          <div className="flex items-center justify-between text-xs text-muted-foreground">
            <span>Downloading update…</span>
            <span className="font-mono">{status.percent}%</span>
          </div>
          <div
            aria-label="Update download progress"
            aria-valuemax={100}
            aria-valuemin={0}
            aria-valuenow={status.percent}
            className="h-1.5 w-full overflow-hidden rounded-full bg-muted"
            role="progressbar"
          >
            <div
              className="h-full rounded-full bg-primary transition-[width] duration-200"
              style={{ width: `${status.percent}%` }}
            />
          </div>
        </div>
      )
    case 'downloaded':
      return (
        <div className="flex flex-col gap-2">
          <Button
            className="w-fit"
            disabled={!isUpdateInstallable(status, captureActive)}
            size="sm"
            onClick={onInstall}
          >
            <ArrowClockwise data-icon="inline-start" />
            Restart &amp; install {status.version}
          </Button>
          <p className="text-xs text-muted-foreground">
            {captureActive
              ? 'Finish your recording first — installing restarts Videorc.'
              : 'Videorc will restart to finish updating.'}
          </p>
        </div>
      )
    case 'not-available':
      return (
        <div className="flex flex-col gap-2">
          <div className="flex items-center gap-1.5 text-xs text-muted-foreground">
            <CheckCircle className="size-3.5 shrink-0 text-success" weight="fill" />
            <span>You’re on the latest version ({status.currentVersion}).</span>
          </div>
          <Button className="w-fit" size="sm" variant="outline" onClick={onCheck}>
            <ArrowClockwise data-icon="inline-start" />
            Check again
          </Button>
        </div>
      )
    case 'error':
      return (
        <div className="flex flex-col gap-2">
          <div className="flex items-start gap-1.5 text-xs text-warning-foreground dark:text-warning">
            <Warning className="size-3.5 shrink-0" weight="fill" />
            <span>Couldn’t check for updates: {status.message}</span>
          </div>
          <Button className="w-fit" size="sm" variant="outline" onClick={onCheck}>
            <ArrowClockwise data-icon="inline-start" />
            Try again
          </Button>
        </div>
      )
    default:
      return (
        <Button className="w-fit" size="sm" variant="outline" onClick={onCheck}>
          <ArrowClockwise data-icon="inline-start" />
          Check for updates
        </Button>
      )
  }
}

function openExternalUrl(url: string): void {
  const opener = window.videorc?.openOAuthUrl
  if (opener) {
    void opener(url)
    return
  }

  window.open(url, '_blank', 'noopener,noreferrer')
}

const PERMISSION_SHORTCUTS: Array<{ label: string; pane: SystemPermissionPane }> = [
  { label: 'Privacy', pane: 'privacy' },
  { label: 'Screen Recording', pane: 'screen-recording' },
  { label: 'Camera', pane: 'camera' },
  { label: 'Microphone', pane: 'microphone' }
]
