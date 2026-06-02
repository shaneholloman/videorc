import {
  Broadcast,
  CheckCircle,
  Gauge,
  LinkSimple,
  SignOut,
  TwitchLogo,
  WarningCircle,
  XLogo,
  YoutubeLogo,
  type Icon
} from '@phosphor-icons/react'
import { useEffect, useMemo, useState, type ReactElement } from 'react'

import { PanelSection } from '@/components/panel-section'
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import { Field, FieldDescription, FieldLabel } from '@/components/ui/field'
import { Input } from '@/components/ui/input'
import { Switch } from '@/components/ui/switch'
import { ToggleGroup, ToggleGroupItem } from '@/components/ui/toggle-group'
import { useStudio } from '@/hooks/use-studio'
import type {
  PlatformAccount,
  StreamAuthMode,
  StreamPlatform,
  StreamTargetRuntime,
  StreamTargetSettings,
  StreamUrlMode
} from '@/lib/backend'
import { isStreamTargetReady } from '@/lib/capture'

type BadgeTone = 'success' | 'warning' | 'destructive' | 'outline'

const PLATFORM_ICON: Record<StreamPlatform, Icon> = {
  youtube: YoutubeLogo,
  twitch: TwitchLogo,
  x: XLogo,
  custom: Broadcast
}

export function StreamingTab(): ReactElement {
  const {
    captureConfig,
    disconnectPlatformAccount,
    patchStreamingTarget,
    platformAccounts,
    health,
    isSessionActive,
    streamTargets,
    stopSession
  } = useStudio()
  const streaming = captureConfig.streaming
  const { video } = captureConfig

  const runtimeById = useMemo(() => {
    const map = new Map<string, StreamTargetRuntime>()
    for (const runtime of streamTargets) {
      map.set(runtime.targetId, runtime)
    }
    return map
  }, [streamTargets])

  const accountByPlatform = useMemo(() => {
    const map = new Map<StreamPlatform, PlatformAccount>()
    for (const account of platformAccounts) {
      map.set(account.platform, account)
    }
    return map
  }, [platformAccounts])

  // A destination is "in trouble" while live if its leg dropped (failed) or it was
  // skipped this session for incomplete credentials (not-configured).
  const problems = streamTargets.filter(
    (runtime) => runtime.state === 'failed' || runtime.state === 'not-configured'
  )

  const [dismissed, setDismissed] = useState(false)
  useEffect(() => {
    if (!isSessionActive) {
      setDismissed(false)
    }
  }, [isSessionActive])

  return (
    <div className="grid gap-4 lg:grid-cols-[minmax(0,1.5fr)_minmax(0,1fr)]">
      <div className="flex flex-col gap-4">
        {isSessionActive && problems.length > 0 && !dismissed ? (
          <StreamFailureBanner
            problems={problems}
            onDismiss={() => setDismissed(true)}
            onStopAll={() => void stopSession()}
          />
        ) : null}
        {isSessionActive ? (
          <p className="text-sm text-muted-foreground">
            Destination credentials are locked while a session is live.
          </p>
        ) : null}
        {streaming.targets.map((target) => (
          <DestinationCard
            account={accountByPlatform.get(target.platform)}
            disabled={isSessionActive}
            key={target.id}
            runtime={runtimeById.get(target.id)}
            target={target}
            onDisconnect={disconnectPlatformAccount}
            onPatch={patchStreamingTarget}
          />
        ))}
      </div>

      <StreamingReadiness
        bitrateKbps={video.bitrateKbps}
        ffmpegReady={Boolean(health?.ffmpeg.available)}
        recordEnabled={captureConfig.recordEnabled}
        targets={streaming.targets}
        video={`${video.width}×${video.height} @ ${video.fps}`}
      />
    </div>
  )
}

function StreamFailureBanner({
  problems,
  onStopAll,
  onDismiss
}: {
  problems: StreamTargetRuntime[]
  onStopAll: () => void
  onDismiss: () => void
}): ReactElement {
  const failed = problems.filter((target) => target.state === 'failed')
  const skipped = problems.filter((target) => target.state === 'not-configured')

  return (
    <div className="flex flex-col gap-3 rounded-lg border border-warning/40 bg-warning/10 p-3">
      <div className="flex items-start gap-2.5">
        <WarningCircle className="size-5 shrink-0 text-warning" weight="fill" />
        <div className="flex flex-col gap-1 text-sm">
          <span className="font-medium">Some destinations aren’t live</span>
          {failed.length ? (
            <span className="text-muted-foreground">
              Stopped: {failed.map((target) => target.label).join(', ')}. The other destinations keep
              streaming.
            </span>
          ) : null}
          {skipped.length ? (
            <span className="text-muted-foreground">
              Skipped:{' '}
              {skipped
                .map((target) => (target.message ? `${target.label} (${target.message})` : target.label))
                .join(', ')}
              .
            </span>
          ) : null}
        </div>
      </div>
      <div className="flex gap-2">
        <Button size="sm" variant="destructive" onClick={onStopAll}>
          Stop all
        </Button>
        <Button size="sm" variant="outline" onClick={onDismiss}>
          Continue streaming
        </Button>
      </div>
    </div>
  )
}

function configuredBadge(enabled: boolean, ready: boolean): { tone: BadgeTone; label: string } {
  if (!enabled) {
    return { tone: 'outline', label: 'Off' }
  }
  return ready ? { tone: 'success', label: 'Ready' } : { tone: 'warning', label: 'Needs setup' }
}

function runtimeBadge(runtime: StreamTargetRuntime): { tone: BadgeTone; label: string } {
  switch (runtime.state) {
    case 'live':
      return { tone: 'success', label: 'On air' }
    case 'connecting':
      return { tone: 'warning', label: 'Connecting' }
    case 'failed':
      return { tone: 'destructive', label: 'Stopped' }
    case 'not-configured':
      return { tone: 'warning', label: 'Skipped' }
    case 'stopped':
      return { tone: 'outline', label: 'Ended' }
    default:
      return { tone: 'outline', label: 'Idle' }
  }
}

function DestinationCard({
  target,
  account,
  disabled,
  runtime,
  onDisconnect,
  onPatch
}: {
  target: StreamTargetSettings
  account?: PlatformAccount
  disabled: boolean
  runtime?: StreamTargetRuntime
  onDisconnect: (platform: StreamPlatform) => void
  onPatch: (targetId: string, patch: Partial<StreamTargetSettings>) => void
}): ReactElement {
  const ready = isStreamTargetReady(target)
  const fullUrl = target.urlMode === 'full-url'
  const nativeDestination = target.platform !== 'custom'
  const oauthMode = nativeDestination && target.authMode === 'oauth'
  // While a session is live the runtime status (on air / stopped / skipped) takes
  // over the badge; otherwise it reflects the saved-credential readiness.
  const badge = runtime ? runtimeBadge(runtime) : configuredBadge(target.enabled, ready)

  return (
    <PanelSection
      action={
        <Switch
          aria-label={`Enable ${target.label}`}
          checked={target.enabled}
          disabled={disabled}
          onCheckedChange={(checked) => onPatch(target.id, { enabled: checked })}
        />
      }
      icon={PLATFORM_ICON[target.platform]}
      title={target.label}
    >
      <div className="flex flex-wrap items-center gap-x-2 gap-y-1">
        <Badge className="w-fit" variant={badge.tone}>
          {badge.label}
        </Badge>
        {runtime?.message ? (
          <span className="text-xs text-muted-foreground">{runtime.message}</span>
        ) : null}
      </div>

      {target.platform === 'custom' ? (
        <Field>
          <FieldLabel>URL mode</FieldLabel>
          <ToggleGroup
            className="w-full"
            disabled={disabled}
            type="single"
            value={target.urlMode ?? 'server-and-key'}
            variant="outline"
            onValueChange={(value) => value && onPatch(target.id, { urlMode: value as StreamUrlMode })}
          >
            <ToggleGroupItem value="server-and-key">Server + key</ToggleGroupItem>
            <ToggleGroupItem value="full-url">Full URL</ToggleGroupItem>
          </ToggleGroup>
        </Field>
      ) : null}

      {nativeDestination ? (
        <Field>
          <FieldLabel>Auth mode</FieldLabel>
          <ToggleGroup
            className="w-full"
            disabled={disabled}
            type="single"
            value={target.authMode}
            variant="outline"
            onValueChange={(value) => value && onPatch(target.id, { authMode: value as StreamAuthMode })}
          >
            <ToggleGroupItem value="oauth">OAuth</ToggleGroupItem>
            <ToggleGroupItem value="manual-rtmp">Manual RTMP</ToggleGroupItem>
          </ToggleGroup>
        </Field>
      ) : null}

      {oauthMode ? (
        <OAuthAccountPanel
          account={account}
          disabled={disabled}
          platform={target.platform}
          onDisconnect={onDisconnect}
        />
      ) : (
        <>
          <Field>
            <FieldLabel htmlFor={`${target.id}-server`}>{fullUrl ? 'Full RTMP URL' : 'RTMP server'}</FieldLabel>
            <Input
              disabled={disabled}
              id={`${target.id}-server`}
              placeholder={fullUrl ? 'rtmp://server/app/key' : 'rtmp://server/app'}
              value={target.serverUrl}
              onChange={(event) => onPatch(target.id, { serverUrl: event.target.value })}
            />
          </Field>

          {!fullUrl ? (
            <Field>
              <FieldLabel htmlFor={`${target.id}-key`}>Stream key</FieldLabel>
              <Input
                autoComplete="off"
                disabled={disabled}
                id={`${target.id}-key`}
                placeholder="paste your stream key"
                type="password"
                value={target.streamKey}
                onChange={(event) => onPatch(target.id, { streamKey: event.target.value })}
              />
              <FieldDescription>
                Saved locally per platform — switching platforms never overwrites another key.
              </FieldDescription>
            </Field>
          ) : null}
        </>
      )}

      {target.platform === 'x' && !oauthMode ? (
        <p className="text-xs text-muted-foreground">
          X needs Media Studio Producer access; copy the RTMP URL and key from a Producer source.
        </p>
      ) : null}
    </PanelSection>
  )
}

function OAuthAccountPanel({
  account,
  disabled,
  platform,
  onDisconnect
}: {
  account?: PlatformAccount
  disabled: boolean
  platform: StreamPlatform
  onDisconnect: (platform: StreamPlatform) => void
}): ReactElement {
  if (!account) {
    return (
      <div className="flex flex-col gap-2 rounded-lg border bg-muted/30 p-3">
        <div className="flex items-center justify-between gap-3">
          <span className="text-sm font-medium">No account connected</span>
          <Button disabled size="sm" variant="secondary">
            <LinkSimple data-icon="inline-start" weight="bold" />
            Connect
          </Button>
        </div>
        <p className="text-xs text-muted-foreground">Provider setup pending.</p>
      </div>
    )
  }

  return (
    <div className="flex flex-col gap-2 rounded-lg border bg-muted/30 p-3">
      <div className="flex items-start justify-between gap-3">
        <div className="flex min-w-0 flex-col gap-1">
          <span className="truncate text-sm font-medium">{account.accountLabel}</span>
          <span className="truncate text-xs text-muted-foreground">
            {account.accountHandle ?? account.accountId}
          </span>
        </div>
        <Badge variant={account.status === 'connected' ? 'success' : 'warning'}>
          {account.status === 'connected' ? 'Connected' : 'Reconnect'}
        </Badge>
      </div>
      <div className="flex flex-wrap gap-1">
        {account.scopes.length ? (
          account.scopes.map((scope) => (
            <Badge key={scope} variant="outline">
              {scope}
            </Badge>
          ))
        ) : (
          <span className="text-xs text-muted-foreground">No granted scopes reported.</span>
        )}
      </div>
      <Button disabled={disabled} size="sm" variant="outline" onClick={() => onDisconnect(platform)}>
        <SignOut data-icon="inline-start" weight="bold" />
        Disconnect
      </Button>
    </div>
  )
}

function StreamingReadiness({
  targets,
  bitrateKbps,
  ffmpegReady,
  recordEnabled,
  video
}: {
  targets: StreamTargetSettings[]
  bitrateKbps: number
  ffmpegReady: boolean
  recordEnabled: boolean
  video: string
}): ReactElement {
  const enabled = targets.filter((target) => target.enabled)
  const readyCount = enabled.filter(isStreamTargetReady).length
  const allReady = enabled.length > 0 && readyCount === enabled.length
  const presetOk = bitrateKbps <= 6000
  const uploadMbps = enabled.length
    ? Math.round((((bitrateKbps + 128) * enabled.length * 1.1) / 1000) * 10) / 10
    : 0
  const diskMbPerMin = Math.round((bitrateKbps / 8 / 1000) * 60)

  return (
    <PanelSection icon={Gauge} title="Multistream readiness">
      <ChecklistRow
        detail={enabled.length ? `${readyCount}/${enabled.length} ready` : 'No destinations enabled'}
        label="Destination credentials saved"
        ok={allReady}
      />
      <ChecklistRow
        detail={`${video} · ${bitrateKbps} kbps${presetOk ? '' : ' · exceeds Twitch ~6000'}`}
        label="Output preset compatible"
        ok={presetOk}
      />
      <ChecklistRow detail={ffmpegReady ? 'ready' : 'check Settings'} label="FFmpeg available" ok={ffmpegReady} />
      <InfoRow
        detail={
          enabled.length
            ? `~${uploadMbps} Mbps to ${enabled.length} destination${enabled.length > 1 ? 's' : ''}`
            : '—'
        }
        label="Estimated upload"
      />
      {recordEnabled ? <InfoRow detail={`~${diskMbPerMin} MB/min`} label="Estimated disk" /> : null}

      <p className="text-xs text-muted-foreground">
        v1 streams the same encode to every destination via FFmpeg, so the bitrate is capped by the
        strictest platform (Twitch ~6000 kbps).
      </p>
    </PanelSection>
  )
}

function ChecklistRow({ label, detail, ok }: { label: string; detail: string; ok: boolean }): ReactElement {
  return (
    <div className="flex items-start justify-between gap-3 text-sm">
      <div className="flex items-center gap-2">
        {ok ? (
          <CheckCircle className="size-4 shrink-0 text-primary" weight="fill" />
        ) : (
          <WarningCircle className="size-4 shrink-0 text-muted-foreground" weight="fill" />
        )}
        <span>{label}</span>
      </div>
      <span className="text-right text-xs text-muted-foreground">{detail}</span>
    </div>
  )
}

function InfoRow({ label, detail }: { label: string; detail: string }): ReactElement {
  return (
    <div className="flex items-center justify-between gap-3 text-sm">
      <span className="text-muted-foreground">{label}</span>
      <span className="text-xs tabular-nums">{detail}</span>
    </div>
  )
}
