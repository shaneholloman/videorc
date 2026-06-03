import {
  ArrowsClockwise,
  Broadcast,
  CheckCircle,
  FloppyDisk,
  Gauge,
  LinkSimple,
  MagnifyingGlass,
  SignOut,
  TextAa,
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
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue
} from '@/components/ui/select'
import { Switch } from '@/components/ui/switch'
import { ToggleGroup, ToggleGroupItem } from '@/components/ui/toggle-group'
import { useStudio } from '@/hooks/use-studio'
import type {
  PlatformAccount,
  PlatformAccountValidation,
  OAuthProviderCredentialStatus,
  StreamAuthMode,
  StreamMetadataDraft,
  StreamMetadataValidation,
  StreamPlatform,
  StreamPrivacy,
  StreamTargetRuntime,
  StreamTargetSettings,
  StreamUrlMode,
  TwitchCategory,
  YouTubeChannel
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
    connectPlatformAccount,
    disconnectPlatformAccount,
    patchStreamMetadataDraft,
    patchStreamTargetMetadataDraft,
    patchStreamingTarget,
    platformAccountValidations,
    platformAccounts,
    youtubeChannels,
    youtubeChannelsLoading,
    refreshYouTubeChannels,
    oauthProviderCredentials,
    saveStreamMetadataDraft,
    selectYouTubeChannel,
    health,
    isSessionActive,
    streamMetadataDraft,
    streamMetadataSavePending,
    streamMetadataValidation,
    streamTargets,
    twitchCategories,
    twitchCategorySearchPending,
    searchTwitchCategories,
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

  const validationByPlatform = useMemo(() => {
    const map = new Map<StreamPlatform, PlatformAccountValidation>()
    for (const validation of platformAccountValidations) {
      map.set(validation.platform, validation)
    }
    return map
  }, [platformAccountValidations])

  const credentialsByPlatform = useMemo(() => {
    const map = new Map<StreamPlatform, OAuthProviderCredentialStatus>()
    for (const status of oauthProviderCredentials) {
      map.set(status.platform, status)
    }
    return map
  }, [oauthProviderCredentials])

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
            credentials={credentialsByPlatform.get(target.platform)}
            disabled={isSessionActive}
            key={target.id}
            runtime={runtimeById.get(target.id)}
            target={target}
            validation={validationByPlatform.get(target.platform)}
            youtubeChannels={youtubeChannels}
            youtubeChannelsLoading={youtubeChannelsLoading}
            onConnect={connectPlatformAccount}
            onDisconnect={disconnectPlatformAccount}
            onPatch={patchStreamingTarget}
            onRefreshYouTubeChannels={refreshYouTubeChannels}
            onSelectYouTubeChannel={selectYouTubeChannel}
          />
        ))}
      </div>

      <div className="flex flex-col gap-4">
        <MetadataEditor
          disabled={isSessionActive}
          draft={streamMetadataDraft}
          pending={streamMetadataSavePending}
          twitchCategories={twitchCategories}
          twitchCategorySearchPending={twitchCategorySearchPending}
          targets={streaming.targets}
          validation={streamMetadataValidation}
          onPatchDraft={patchStreamMetadataDraft}
          onPatchTarget={patchStreamTargetMetadataDraft}
          onSave={() => void saveStreamMetadataDraft()}
          onSearchTwitchCategories={searchTwitchCategories}
        />
        <StreamingReadiness
          bitrateKbps={video.bitrateKbps}
          ffmpegReady={Boolean(health?.ffmpeg.available)}
          recordEnabled={captureConfig.recordEnabled}
          targets={streaming.targets}
          video={`${video.width}×${video.height} @ ${video.fps}`}
        />
      </div>
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
  credentials,
  disabled,
  runtime,
  validation,
  youtubeChannels,
  youtubeChannelsLoading,
  onConnect,
  onDisconnect,
  onPatch,
  onRefreshYouTubeChannels,
  onSelectYouTubeChannel
}: {
  target: StreamTargetSettings
  account?: PlatformAccount
  credentials?: OAuthProviderCredentialStatus
  disabled: boolean
  runtime?: StreamTargetRuntime
  validation?: PlatformAccountValidation
  youtubeChannels: YouTubeChannel[]
  youtubeChannelsLoading: boolean
  onConnect: (platform: StreamPlatform) => void
  onDisconnect: (platform: StreamPlatform) => void
  onPatch: (targetId: string, patch: Partial<StreamTargetSettings>) => void
  onRefreshYouTubeChannels: (accountId?: string) => Promise<void>
  onSelectYouTubeChannel: (channelId: string, accountId?: string) => Promise<void>
}): ReactElement {
  const ready = isStreamTargetReady(target)
  const fullUrl = target.urlMode === 'full-url'
  const nativeDestination = target.platform !== 'custom'
  const oauthMode = nativeDestination && target.authMode === 'oauth'
  // While a session is live the runtime status (on air / stopped / skipped) takes
  // over the badge; otherwise it reflects the saved-credential readiness.
  const savedStatusBadge = target.status ? streamTargetStatusBadge(target.status.state) : null
  const badge = runtime ? runtimeBadge(runtime) : (savedStatusBadge ?? configuredBadge(target.enabled, ready))
  const statusMessage = runtime?.message ?? target.status?.message

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
        {statusMessage ? (
          <span className="text-xs text-muted-foreground">{statusMessage}</span>
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
          credentials={credentials}
          disabled={disabled}
          platform={target.platform}
          validation={validation}
          youtubeChannels={youtubeChannels}
          youtubeChannelsLoading={youtubeChannelsLoading}
          onConnect={onConnect}
          onDisconnect={onDisconnect}
          onRefreshYouTubeChannels={onRefreshYouTubeChannels}
          onSelectYouTubeChannel={onSelectYouTubeChannel}
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

function streamTargetStatusBadge(state: NonNullable<StreamTargetSettings['status']>['state']): {
  tone: BadgeTone
  label: string
} {
  switch (state) {
    case 'ready':
      return { tone: 'success', label: 'Prepared' }
    case 'connecting':
      return { tone: 'warning', label: 'Updating' }
    case 'live':
      return { tone: 'success', label: 'On air' }
    case 'warning':
      return { tone: 'warning', label: 'Review' }
    case 'failed':
      return { tone: 'destructive', label: 'Failed' }
    case 'stopped':
      return { tone: 'outline', label: 'Ended' }
    default:
      return { tone: 'outline', label: 'Idle' }
  }
}

function OAuthAccountPanel({
  account,
  credentials,
  disabled,
  platform,
  validation,
  youtubeChannels,
  youtubeChannelsLoading,
  onConnect,
  onDisconnect,
  onRefreshYouTubeChannels,
  onSelectYouTubeChannel
}: {
  account?: PlatformAccount
  credentials?: OAuthProviderCredentialStatus
  disabled: boolean
  platform: StreamPlatform
  validation?: PlatformAccountValidation
  youtubeChannels: YouTubeChannel[]
  youtubeChannelsLoading: boolean
  onConnect: (platform: StreamPlatform) => void
  onDisconnect: (platform: StreamPlatform) => void
  onRefreshYouTubeChannels: (accountId?: string) => Promise<void>
  onSelectYouTubeChannel: (channelId: string, accountId?: string) => Promise<void>
}): ReactElement {
  if (!account) {
    const connectDisabled = disabled || credentials?.ready === false
    return (
      <div className="flex flex-col gap-2 rounded-lg border bg-muted/30 p-3">
        <div className="flex items-center justify-between gap-3">
          <span className="text-sm font-medium">No account connected</span>
          <Button disabled={connectDisabled} size="sm" variant="secondary" onClick={() => onConnect(platform)}>
            <LinkSimple data-icon="inline-start" weight="bold" />
            Connect
          </Button>
        </div>
        <p className="text-xs text-muted-foreground">
          {credentials?.message ?? 'Uses backend provider credentials.'}
        </p>
        {credentials ? (
          <Badge className="w-fit" variant={credentials.ready ? 'outline' : 'warning'}>
            {credentialSourceLabel(credentials)}
          </Badge>
        ) : null}
      </div>
    )
  }

  const youtubeChannelOptions =
    platform === 'youtube' && !youtubeChannels.some((channel) => channel.channelId === account.accountId)
      ? [
          {
            channelId: account.accountId,
            title: account.accountLabel,
            handle: account.accountHandle,
            avatarUrl: account.avatarUrl
          },
          ...youtubeChannels
        ]
      : youtubeChannels

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
      {validation ? (
        <div className="flex flex-col gap-1 rounded-md bg-background/60 px-2 py-1.5">
          <Badge className="w-fit" variant={validationBadge(validation).tone}>
            {validationBadge(validation).label}
          </Badge>
          <span className="text-xs text-muted-foreground">{validation.message}</span>
        </div>
      ) : null}
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
      {platform === 'youtube' ? (
        <Field>
          <FieldLabel>YouTube channel</FieldLabel>
          <div className="flex gap-2">
            <Select
              disabled={disabled || youtubeChannelsLoading || youtubeChannelOptions.length === 0}
              value={account.accountId}
              onValueChange={(channelId) => void onSelectYouTubeChannel(channelId, account.accountId)}
            >
              <SelectTrigger className="min-w-0 flex-1">
                <SelectValue placeholder={youtubeChannelsLoading ? 'Loading channels' : 'Select channel'} />
              </SelectTrigger>
              <SelectContent>
                {youtubeChannelOptions.map((channel) => (
                  <SelectItem key={channel.channelId} value={channel.channelId}>
                    {channel.title}
                    {channel.handle ? ` (${channel.handle})` : ''}
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
            <Button
              disabled={disabled || youtubeChannelsLoading}
              size="sm"
              variant="outline"
              onClick={() => void onRefreshYouTubeChannels(account.accountId)}
            >
              <ArrowsClockwise data-icon="inline-start" weight="bold" />
              {youtubeChannelsLoading ? 'Loading' : 'Refresh'}
            </Button>
          </div>
          <FieldDescription>
            {youtubeChannels.length
              ? 'Switching channels clears prepared YouTube ingest state for the previous channel.'
              : 'Refresh after connecting to load channels available to this Google account.'}
          </FieldDescription>
        </Field>
      ) : null}
      <Button disabled={disabled} size="sm" variant="outline" onClick={() => onDisconnect(platform)}>
        <SignOut data-icon="inline-start" weight="bold" />
        Disconnect
      </Button>
    </div>
  )
}

function credentialSourceLabel(credentials: OAuthProviderCredentialStatus): string {
  switch (credentials.clientIdSource) {
    case 'environment':
      return 'Environment override'
    case 'bundled':
      return 'Bundled default'
    case 'missing':
      return 'Missing client ID'
  }
}

function validationBadge(validation: PlatformAccountValidation): { tone: BadgeTone; label: string } {
  switch (validation.state) {
    case 'valid':
      return { tone: 'success', label: 'Validated' }
    case 'refreshed':
      return { tone: 'success', label: 'Refreshed' }
    case 'needs-reconnect':
      return { tone: 'warning', label: 'Needs reconnect' }
    default:
      return { tone: 'outline', label: 'Not checked' }
  }
}

function MetadataEditor({
  draft,
  validation,
  targets,
  disabled,
  pending,
  twitchCategories,
  twitchCategorySearchPending,
  onPatchDraft,
  onPatchTarget,
  onSave,
  onSearchTwitchCategories
}: {
  draft: StreamMetadataDraft | null
  validation: StreamMetadataValidation | null
  targets: StreamTargetSettings[]
  disabled: boolean
  pending: boolean
  twitchCategories: TwitchCategory[]
  twitchCategorySearchPending: boolean
  onPatchDraft: (patch: Partial<StreamMetadataDraft>) => void
  onPatchTarget: (
    platform: StreamMetadataDraft['targetOverrides'][number]['platform'],
    patch: Partial<StreamMetadataDraft['targetOverrides'][number]>
  ) => void
  onSave: () => void
  onSearchTwitchCategories: (query: string) => Promise<void>
}): ReactElement {
  const nativeTargets = targets.filter((target) => target.platform !== 'custom')
  const globalTitleIssue = metadataIssue(validation, 'title')

  return (
    <PanelSection
      action={
        <Button disabled={!draft || pending} size="sm" variant="secondary" onClick={onSave}>
          <FloppyDisk data-icon="inline-start" weight="bold" />
          {pending ? 'Saving' : 'Save'}
        </Button>
      }
      icon={TextAa}
      title="Broadcast metadata"
    >
      {!draft ? (
        <p className="text-sm text-muted-foreground">Loading metadata draft.</p>
      ) : (
        <>
          <Field>
            <FieldLabel htmlFor="stream-title">Title</FieldLabel>
            <Input
              aria-invalid={Boolean(globalTitleIssue)}
              disabled={disabled}
              id="stream-title"
              placeholder="Untitled livestream"
              value={draft.title}
              onChange={(event) => onPatchDraft({ title: event.target.value })}
            />
            {globalTitleIssue ? <FieldDescription>{globalTitleIssue.message}</FieldDescription> : null}
          </Field>

          <Field>
            <FieldLabel htmlFor="stream-description">Description</FieldLabel>
            <textarea
              className="min-h-24 w-full resize-y rounded-lg border border-transparent bg-input/50 px-3 py-2 text-sm outline-none transition-[border-color,box-shadow] focus-visible:border-ring focus-visible:ring-3 focus-visible:ring-ring/30 disabled:cursor-not-allowed disabled:opacity-50"
              disabled={disabled}
              id="stream-description"
              placeholder="Optional"
              value={draft.description}
              onChange={(event) => onPatchDraft({ description: event.target.value })}
            />
          </Field>

          <Field>
            <FieldLabel>Default privacy</FieldLabel>
            <Select
              disabled={disabled}
              value={draft.defaultPrivacy}
              onValueChange={(value) => onPatchDraft({ defaultPrivacy: value as StreamPrivacy })}
            >
              <SelectTrigger className="w-full">
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                <SelectItem value="private">Private</SelectItem>
                <SelectItem value="unlisted">Unlisted</SelectItem>
                <SelectItem value="public">Public</SelectItem>
              </SelectContent>
            </Select>
          </Field>

          <div className="flex flex-col gap-4">
            {draft.targetOverrides.map((override) => {
              const target = nativeTargets.find((item) => item.platform === override.platform)
              return (
                <MetadataOverride
                  disabled={disabled}
                  draft={draft}
                  key={override.platform}
                  label={target?.label ?? platformLabel(override.platform)}
                  override={override}
                  twitchCategories={twitchCategories}
                  twitchCategorySearchPending={twitchCategorySearchPending}
                  validation={validation}
                  onPatch={(patch) => onPatchTarget(override.platform, patch)}
                  onSearchTwitchCategories={onSearchTwitchCategories}
                />
              )
            })}
          </div>

          {validation && !validation.valid ? (
            <div className="rounded-lg border border-warning/40 bg-warning/10 p-3 text-xs text-muted-foreground">
              {validation.issues.length} metadata warning{validation.issues.length === 1 ? '' : 's'} before Go Live.
            </div>
          ) : (
            <Badge className="w-fit" variant="success">
              Metadata ready
            </Badge>
          )}
        </>
      )}
    </PanelSection>
  )
}

function MetadataOverride({
  override,
  draft,
  label,
  disabled,
  twitchCategories,
  twitchCategorySearchPending,
  validation,
  onPatch,
  onSearchTwitchCategories
}: {
  override: StreamMetadataDraft['targetOverrides'][number]
  draft: StreamMetadataDraft
  label: string
  disabled: boolean
  twitchCategories: TwitchCategory[]
  twitchCategorySearchPending: boolean
  validation: StreamMetadataValidation | null
  onPatch: (patch: Partial<StreamMetadataDraft['targetOverrides'][number]>) => void
  onSearchTwitchCategories: (query: string) => Promise<void>
}): ReactElement {
  const titleIssue = metadataIssue(validation, 'title', override.platform)
  const fieldsDisabled = disabled || !override.customize
  const twitch = override.platform === 'twitch'
  const youtube = override.platform === 'youtube'
  const x = override.platform === 'x'
  const [twitchCategoryQuery, setTwitchCategoryQuery] = useState(override.twitchCategoryName ?? '')
  const twitchCategoryOptions =
    twitch && override.twitchCategoryId && !twitchCategories.some((category) => category.id === override.twitchCategoryId)
      ? [
          {
            id: override.twitchCategoryId,
            name: override.twitchCategoryName ?? override.twitchCategoryId
          },
          ...twitchCategories
        ]
      : twitchCategories

  useEffect(() => {
    setTwitchCategoryQuery(override.twitchCategoryName ?? '')
  }, [override.twitchCategoryName])

  return (
    <div className="flex flex-col gap-3 border-t pt-4 first:border-t-0 first:pt-0">
      <div className="flex items-center justify-between gap-3">
        <div className="flex min-w-0 flex-col">
          <span className="truncate text-sm font-medium">{label}</span>
          <span className="truncate text-xs text-muted-foreground">
            {override.customize ? 'Custom metadata' : 'Inherits global metadata'}
          </span>
        </div>
        <Switch
          aria-label={`Customize ${label}`}
          checked={override.customize}
          disabled={disabled}
          onCheckedChange={(customize) => onPatch({ customize })}
        />
      </div>

      <Field>
        <FieldLabel htmlFor={`${override.platform}-metadata-title`}>Title</FieldLabel>
        <Input
          aria-invalid={Boolean(titleIssue)}
          disabled={fieldsDisabled}
          id={`${override.platform}-metadata-title`}
          placeholder={draft.title || 'Inherits global title'}
          value={override.title}
          onChange={(event) => onPatch({ title: event.target.value })}
        />
        {titleIssue ? <FieldDescription>{titleIssue.message}</FieldDescription> : null}
      </Field>

      <Field>
        <FieldLabel htmlFor={`${override.platform}-metadata-description`}>Description</FieldLabel>
        <textarea
          className="min-h-20 w-full resize-y rounded-lg border border-transparent bg-input/50 px-3 py-2 text-sm outline-none transition-[border-color,box-shadow] focus-visible:border-ring focus-visible:ring-3 focus-visible:ring-ring/30 disabled:cursor-not-allowed disabled:opacity-50"
          disabled={fieldsDisabled || twitch}
          id={`${override.platform}-metadata-description`}
          placeholder={twitch ? 'Not supported by Twitch' : draft.description || 'Inherits global description'}
          value={twitch ? '' : override.description}
          onChange={(event) => onPatch({ description: event.target.value })}
        />
        {twitch ? <FieldDescription>Twitch supports title, category, and language.</FieldDescription> : null}
      </Field>

      {youtube ? (
        <div className="grid gap-3 sm:grid-cols-2">
          <Field>
            <FieldLabel>Privacy</FieldLabel>
            <Select
              disabled={fieldsDisabled}
              value={override.privacy}
              onValueChange={(value) => onPatch({ privacy: value as StreamPrivacy })}
            >
              <SelectTrigger className="w-full">
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                <SelectItem value="private">Private</SelectItem>
                <SelectItem value="unlisted">Unlisted</SelectItem>
                <SelectItem value="public">Public</SelectItem>
              </SelectContent>
            </Select>
          </Field>
          <Field>
            <FieldLabel>Made for kids</FieldLabel>
            <ToggleGroup
              className="w-full"
              disabled={fieldsDisabled}
              type="single"
              value={override.youtubeMadeForKids ? 'yes' : 'no'}
              variant="outline"
              onValueChange={(value) => value && onPatch({ youtubeMadeForKids: value === 'yes' })}
            >
              <ToggleGroupItem value="no">No</ToggleGroupItem>
              <ToggleGroupItem value="yes">Yes</ToggleGroupItem>
            </ToggleGroup>
          </Field>
        </div>
      ) : null}

      {twitch ? (
        <div className="grid gap-3 sm:grid-cols-2">
          <Field>
            <FieldLabel htmlFor="twitch-category">Category</FieldLabel>
            <div className="flex gap-2">
              <Input
                disabled={fieldsDisabled}
                id="twitch-category"
                placeholder="Just Chatting"
                value={twitchCategoryQuery}
                onChange={(event) => {
                  setTwitchCategoryQuery(event.target.value)
                  onPatch({ twitchCategoryId: undefined, twitchCategoryName: event.target.value })
                }}
              />
              <Button
                disabled={fieldsDisabled || twitchCategorySearchPending || twitchCategoryQuery.trim().length < 2}
                size="sm"
                variant="outline"
                onClick={() => void onSearchTwitchCategories(twitchCategoryQuery)}
              >
                <MagnifyingGlass data-icon="inline-start" weight="bold" />
                {twitchCategorySearchPending ? 'Searching' : 'Search'}
              </Button>
            </div>
            {twitchCategoryOptions.length ? (
              <Select
                disabled={fieldsDisabled || twitchCategorySearchPending}
                value={override.twitchCategoryId ?? ''}
                onValueChange={(categoryId) => {
                  const category = twitchCategoryOptions.find((item) => item.id === categoryId)
                  if (category) {
                    setTwitchCategoryQuery(category.name)
                    onPatch({ twitchCategoryId: category.id, twitchCategoryName: category.name })
                  }
                }}
              >
                <SelectTrigger className="w-full">
                  <SelectValue placeholder="Select category" />
                </SelectTrigger>
                <SelectContent>
                  {twitchCategoryOptions.map((category) => (
                    <SelectItem key={category.id} value={category.id}>
                      {category.name}
                    </SelectItem>
                  ))}
                </SelectContent>
              </Select>
            ) : null}
          </Field>
          <Field>
            <FieldLabel htmlFor="twitch-language">Language</FieldLabel>
            <Input
              disabled={fieldsDisabled}
              id="twitch-language"
              placeholder="en"
              value={override.twitchLanguage ?? ''}
              onChange={(event) => onPatch({ twitchLanguage: event.target.value })}
            />
          </Field>
        </div>
      ) : null}

      {x ? (
        <Field>
          <FieldLabel>Visibility</FieldLabel>
          <Select
            disabled={fieldsDisabled}
            value={override.xVisibility ?? 'public'}
            onValueChange={(value) => onPatch({ xVisibility: value as StreamPrivacy })}
          >
            <SelectTrigger className="w-full">
              <SelectValue />
            </SelectTrigger>
            <SelectContent>
              <SelectItem value="public">Public</SelectItem>
              <SelectItem value="unlisted">Unlisted</SelectItem>
              <SelectItem value="private">Private</SelectItem>
            </SelectContent>
          </Select>
        </Field>
      ) : null}
    </div>
  )
}

function metadataIssue(
  validation: StreamMetadataValidation | null,
  field: string,
  platform?: StreamPlatform
): StreamMetadataValidation['issues'][number] | undefined {
  return validation?.issues.find((issue) => issue.field === field && issue.platform === platform)
}

function platformLabel(platform: StreamPlatform): string {
  switch (platform) {
    case 'youtube':
      return 'YouTube'
    case 'twitch':
      return 'Twitch'
    case 'x':
      return 'X'
    default:
      return 'Custom'
  }
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
