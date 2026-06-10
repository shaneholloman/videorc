import {
  Broadcast,
  ChatCircle,
  CheckCircle,
  FolderOpen,
  type Icon,
  ImageSquare,
  Layout,
  Monitor,
  Record,
  SpeakerHigh,
  SpeakerSlash,
  StopCircle,
  WarningCircle
} from '@phosphor-icons/react'
import { useEffect, useState, type ReactElement } from 'react'

import { BlockingBanner } from '@/components/blocking-banner'
import { LiveChatPanel } from '@/components/live-chat-panel'
import { PreviewStage } from '@/components/preview-stage'
import { StatusBadge } from '@/components/status-badge'
import {
  Accordion,
  AccordionContent,
  AccordionItem,
  AccordionTrigger
} from '@/components/ui/accordion'
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle
} from '@/components/ui/dialog'
import { Field, FieldContent, FieldLabel } from '@/components/ui/field'
import { Input } from '@/components/ui/input'
import { ScrollArea } from '@/components/ui/scroll-area'
import { Separator } from '@/components/ui/separator'
import { Switch } from '@/components/ui/switch'
import { useWorkspaceNav, type StudioPanel, type WorkspaceTab } from '@/components/workspace-nav'
import { useStudio } from '@/hooks/use-studio'
import type { GoLiveDestinationPreflight, StreamPlatform, StreamScreen } from '@/lib/backend'
import { videoProfileCompatibility } from '@/lib/capture'
import { studioHealth } from '@/lib/studio-health'
import { cn } from '@/lib/utils'

export function StudioTab(): ReactElement {
  const studio = useStudio()
  const { openStudioPanel } = useWorkspaceNav()
  const {
    recording,
    elapsed,
    canStart,
    canStop,
    startRequestPending,
    stopRequestPending,
    startBlockedReason,
    visibleStartBlockedReason,
    startSession,
    stopSession,
    captureConfig,
    setCaptureConfig,
    previewUrl,
    previewLoading,
    previewLiveStatus,
    previewCameraStatus,
    previewScreenStatus,
    previewSurfaceStatus,
    nativePreviewSurfaceEnabled,
    refreshPreview,
    registerPreviewSurfaceResize,
    syncNativePreviewSurfaceBounds,
    openPreviewPermissions,
    revealPermissionTarget,
    runtimeInfo,
    selectedCaptureDevice,
    selectedCamera,
    selectedMicrophone,
    streamReady,
    wsStatus,
    health,
    diagnosticStats,
    audioMeter,
    meterLevel,
    scene,
    sceneEditMode,
    selectedSceneSourceId,
    setSceneEditMode,
    setSelectedSceneSourceId,
    commitCameraTransform,
    isSessionActive,
    screens,
    activeScreen,
    activateScreen,
    clearActiveScreen,
    goLiveConfirmationOpen,
    goLiveConfirmationPending,
    goLivePartialSetup,
    goLivePreflight,
    streamMetadataDraft,
    patchStreamMetadataDraft,
    cancelGoLiveConfirmation,
    confirmGoLive,
    continueGoLiveWithReadyDestinations
  } = studio

  const active = recording.state === 'recording' || recording.state === 'streaming'
  const previewHealth = studioHealth(diagnosticStats, active)
  const banner = studioBlocker(studio)
  const audioSummary =
    recording.audioTracks?.map((track) => track.label).join(' + ') ?? (selectedMicrophone ? 'Microphone' : 'None')
  const pipelineSummary = recording.pipeline ? pipelineStatusLabel(recording.pipeline.finalization) : 'Ready'
  const liveStreamCompatibility = videoProfileCompatibility({ ...captureConfig, streamEnabled: true })
  const liveStreamBlockedReason = liveStreamCompatibility.blockingReason

  // Two-button start: set the intended mode, then start on the next render so startSession
  // sees the updated streamEnabled (record vs go-live) instead of a stale closure value.
  const [pendingStart, setPendingStart] = useState(false)
  useEffect(() => {
    if (!pendingStart) {
      return
    }
    setPendingStart(false)
    void startSession()
  }, [pendingStart, startSession])

  const handleRecord = (): void => {
    setCaptureConfig((current) => ({ ...current, recordEnabled: true, streamEnabled: false }))
    setPendingStart(true)
  }
  const handleLiveStream = (): void => {
    if (liveStreamBlockedReason) {
      return
    }
    setCaptureConfig((current) => ({ ...current, streamEnabled: true }))
    setPendingStart(true)
  }

  const stopLabel = stopRequestPending
    ? 'Stopping…'
    : recording.state === 'stopping'
      ? 'Force stop'
      : recording.state === 'streaming'
        ? 'End livestream'
        : 'Stop recording'

  return (
    <div className="flex flex-col gap-4">
      <GoLiveConfirmationDialog
        draft={streamMetadataDraft}
        open={goLiveConfirmationOpen}
        pending={goLiveConfirmationPending || startRequestPending}
        preflight={goLivePreflight}
        partialSetup={goLivePartialSetup}
        onCancel={cancelGoLiveConfirmation}
        onConfirm={() => void confirmGoLive()}
        onContinuePartial={() => void continueGoLiveWithReadyDestinations()}
        onPatchDraft={patchStreamMetadataDraft}
      />

      {visibleStartBlockedReason && banner ? (
        <BlockingBanner
          description={visibleStartBlockedReason}
          jumpLabel={banner.jumpLabel}
          jumpTo={banner.jumpTo}
          title={banner.title}
          tone="warning"
        />
      ) : null}

      {/* Big preview on top */}
      <PreviewStage
        activeScreen={activeScreen}
        layout={captureConfig.layout}
        onOpenPermissions={openPreviewPermissions}
        onRevealPermissionTarget={revealPermissionTarget}
        onRetry={refreshPreview}
        onPreviewSurfaceResize={registerPreviewSurfaceResize}
        onNativePreviewSurfaceBounds={syncNativePreviewSurfaceBounds}
        previewCameraStatus={previewCameraStatus}
        previewLiveStatus={previewLiveStatus}
        previewScreenStatus={previewScreenStatus}
        previewSurfaceStatus={previewSurfaceStatus}
        nativePreviewSurfaceEnabled={nativePreviewSurfaceEnabled}
        previewLoading={previewLoading}
        previewUrl={previewUrl}
        runtimeInfo={runtimeInfo}
        scene={scene}
        sceneEditMode={sceneEditMode}
        selectedSceneSourceId={selectedSceneSourceId}
        onSelectSceneSource={setSelectedSceneSourceId}
        onCameraDragCommit={commitCameraTransform}
        dragDisabled={isSessionActive || captureConfig.layout.layoutPreset !== 'screen-camera'}
      />

      {/* Action bar: status + the two primary buttons + output */}
      <div className="flex flex-col gap-3 rounded-2xl border bg-card p-4">
        <div className="flex items-center justify-between gap-3">
          <div className="flex items-center gap-2.5">
            <span
              className={cn(
                'size-2.5 shrink-0 rounded-full',
                recording.state === 'recording' && 'bg-destructive',
                recording.state === 'streaming' && 'bg-success',
                (recording.state === 'starting' || recording.state === 'stopping') && 'bg-warning',
                recording.state === 'failed' && 'bg-destructive',
                recording.state === 'idle' && 'bg-muted-foreground/40',
                active && 'animate-pulse'
              )}
            />
            <div className="flex flex-col">
              <span className="text-sm font-semibold capitalize">{recording.state}</span>
              <span className="text-xs text-muted-foreground">{recording.message ?? 'Idle'}</span>
            </div>
            {previewHealth.tone !== 'neutral' ? (
              <StatusBadge
                label="Preview"
                tone={previewHealth.tone}
                value={previewHealth.value}
              />
            ) : null}
          </div>
          <time className="font-heading text-2xl font-semibold tabular-nums">{elapsed}</time>
        </div>

        {previewHealth.tone === 'error' && previewHealth.detail ? (
          <div className="flex items-center gap-2 rounded-lg border border-destructive/30 bg-destructive/10 px-3 py-1.5 text-xs font-medium text-destructive">
            <WarningCircle className="size-4 shrink-0" weight="fill" />
            <span className="min-w-0">{previewHealth.detail}</span>
          </div>
        ) : null}

        {active ? (
          <Button size="lg" variant="destructive" disabled={!canStop} onClick={stopSession}>
            <StopCircle data-icon="inline-start" weight="fill" />
            {stopLabel}
          </Button>
        ) : (
          <div className="grid grid-cols-1 gap-2 sm:grid-cols-2">
            <Button
              size="lg"
              disabled={!canStart || startRequestPending}
              title={startBlockedReason ?? 'Record to a file'}
              onClick={handleRecord}
            >
              <Record data-icon="inline-start" weight="fill" />
              {startRequestPending ? 'Starting…' : 'Record'}
            </Button>
            <Button
              size="lg"
              variant="outline"
              disabled={wsStatus !== 'connected' || startRequestPending || Boolean(liveStreamBlockedReason)}
              title={liveStreamBlockedReason ?? 'Start livestream'}
              onClick={handleLiveStream}
            >
              <Broadcast data-icon="inline-start" weight="fill" />
              Live Stream
            </Button>
          </div>
        )}
        {!active && liveStreamBlockedReason ? (
          <div className="flex items-center gap-2 rounded-lg border border-warning/30 bg-warning/10 px-3 py-1.5 text-xs font-medium text-warning-foreground dark:text-warning">
            <WarningCircle className="size-4 shrink-0" weight="fill" />
            <span>{liveStreamBlockedReason}</span>
          </div>
        ) : null}

        <div className="flex items-center gap-2 rounded-lg border bg-muted/40 px-3 py-2 text-xs text-muted-foreground">
          <FolderOpen className="size-4 shrink-0" weight="duotone" />
          <span className="truncate">
            {recording.outputPath ?? recording.streamUrl ?? 'Output appears after session start.'}
          </span>
        </div>
      </div>

      {/* All settings, tucked into accordions */}
      <Accordion type="multiple" defaultValue={['source']} className="bg-card">
        <AccordionItem value="source">
          <AccordionTrigger>
            <SectionLabel
              icon={Monitor}
              title="Source"
              summary={selectedCaptureDevice?.name ?? selectedCamera?.name ?? 'No source'}
            />
          </AccordionTrigger>
          <AccordionContent className="flex flex-col gap-3.5">
            <dl className="grid grid-cols-[auto_1fr] gap-x-4 gap-y-1.5 text-sm">
              <SummaryRow label="Screen" value={selectedCaptureDevice?.name ?? 'None'} />
              <SummaryRow label="Camera" value={selectedCamera?.name ?? 'Off'} />
              <SummaryRow label="Microphone" value={selectedMicrophone?.name ?? 'Off'} />
            </dl>
            <Button size="sm" variant="outline" className="w-fit" onClick={() => openStudioPanel('sources')}>
              Configure sources
            </Button>
          </AccordionContent>
        </AccordionItem>

        <AccordionItem value="layout">
          <AccordionTrigger>
            <SectionLabel
              icon={Layout}
              title="Scene & layout"
              summary={captureConfig.layout.layoutPreset.replace(/-/g, ' ')}
            />
          </AccordionTrigger>
          <AccordionContent className="flex flex-col gap-3.5">
            <p className="text-sm text-muted-foreground">
              Camera and screen arrangement, crop, and output resolution.
            </p>
            <Field orientation="horizontal">
              <FieldContent>
                <FieldLabel htmlFor="studio-edit-mode">Edit transforms</FieldLabel>
              </FieldContent>
              <Switch checked={sceneEditMode} id="studio-edit-mode" onCheckedChange={setSceneEditMode} />
            </Field>
            <Button
              size="sm"
              variant="outline"
              className="w-fit"
              data-videorc-open-tab="layout"
              onClick={() => openStudioPanel('layouts')}
            >
              Edit layout
            </Button>
          </AccordionContent>
        </AccordionItem>

        <AccordionItem value="screens">
          <AccordionTrigger>
            <SectionLabel icon={ImageSquare} title="Screens" />
          </AccordionTrigger>
          <AccordionContent className="flex flex-col gap-3.5">
            <StudioScreensRow
              activeScreen={activeScreen}
              screens={screens}
              onActivate={(screenId) => void activateScreen(screenId)}
              onClear={() => void clearActiveScreen()}
              onOpenScreens={() => openStudioPanel('layouts')}
            />
          </AccordionContent>
        </AccordionItem>

        <AccordionItem value="mixer">
          <AccordionTrigger>
            <SectionLabel
              icon={selectedMicrophone ? SpeakerHigh : SpeakerSlash}
              title="Audio mixer"
              summary={selectedMicrophone?.name}
            />
          </AccordionTrigger>
          <AccordionContent className="flex flex-col gap-3.5">
            <MixerRow
              gainDb={captureConfig.audio.microphoneGainDb}
              meterLevel={meterLevel}
              muted={captureConfig.audio.microphoneMuted}
              peakDb={audioMeter?.peakDb}
              selectedMicrophoneName={selectedMicrophone?.name}
              syncOffsetMs={captureConfig.audio.microphoneSyncOffsetMs}
            />
          </AccordionContent>
        </AccordionItem>

        <AccordionItem value="output">
          <AccordionTrigger>
            <SectionLabel
              icon={Broadcast}
              title="Output & status"
              summary={`${captureConfig.video.width}×${captureConfig.video.height} · ${captureConfig.video.fps}fps`}
            />
          </AccordionTrigger>
          <AccordionContent className="flex flex-col gap-3.5">
            <Field orientation="horizontal">
              <FieldContent>
                <FieldLabel htmlFor="studio-record">Record to file</FieldLabel>
              </FieldContent>
              <Switch
                checked={captureConfig.recordEnabled}
                id="studio-record"
                onCheckedChange={(checked) =>
                  setCaptureConfig((current) => ({ ...current, recordEnabled: checked }))
                }
              />
            </Field>
            <dl className="grid grid-cols-[auto_1fr] gap-x-4 gap-y-1.5 text-sm">
              <SummaryRow label="Screen" value={selectedCaptureDevice?.name ?? 'None'} />
              <SummaryRow label="Screen takeover" value={activeScreen?.name ?? 'Normal'} />
              <SummaryRow label="Camera" value={selectedCamera?.name ?? 'Off'} />
              <SummaryRow label="Audio" value={audioSummary} />
              <SummaryRow
                label="Output"
                value={`${captureConfig.video.width}×${captureConfig.video.height} · ${captureConfig.video.fps}fps`}
              />
              <SummaryRow
                label="Mode"
                value={[captureConfig.recordEnabled && 'Record', captureConfig.streamEnabled && 'Stream']
                  .filter(Boolean)
                  .join(' + ') || 'None'}
              />
              <SummaryRow label="Pipeline" value={pipelineSummary} />
            </dl>
            <Separator />
            <div className="flex flex-wrap items-center gap-2">
              <StatusBadge label="Socket" tone={wsStatus === 'connected' ? 'good' : 'warn'} value={wsStatus} />
              <StatusBadge
                label="FFmpeg"
                tone={health?.ffmpeg.available ? 'good' : 'warn'}
                value={health?.ffmpeg.available ? 'ready' : 'check'}
              />
              {captureConfig.streamEnabled ? (
                <StatusBadge label="Stream" tone={streamReady ? 'good' : 'warn'} value={streamReady ? 'ready' : 'setup'} />
              ) : null}
            </div>
            <div className="flex flex-wrap gap-2">
              <Button size="sm" variant="outline" onClick={() => openStudioPanel('recording')}>
                Recording settings
              </Button>
              <Button size="sm" variant="outline" onClick={() => openStudioPanel('live')}>
                Streaming settings
              </Button>
            </div>
          </AccordionContent>
        </AccordionItem>

        <AccordionItem value="chat">
          <AccordionTrigger>
            <SectionLabel icon={ChatCircle} title="Live chat" />
          </AccordionTrigger>
          <AccordionContent className="flex flex-col gap-3.5">
            <LiveChatPanel snapshot={studio.liveChatSnapshot} onClearLocal={studio.clearLiveChat} />
          </AccordionContent>
        </AccordionItem>
      </Accordion>
    </div>
  )
}

function SectionLabel({
  icon: LeadingIcon,
  title,
  summary
}: {
  icon: Icon
  title: string
  summary?: string
}): ReactElement {
  return (
    <span className="flex min-w-0 items-center gap-2.5">
      <LeadingIcon className="size-4 shrink-0 text-muted-foreground" weight="duotone" />
      <span className="font-medium">{title}</span>
      {summary ? (
        <span className="min-w-0 truncate text-xs font-normal text-muted-foreground">{summary}</span>
      ) : null}
    </span>
  )
}

function GoLiveConfirmationDialog({
  open,
  pending,
  partialSetup,
  preflight,
  draft,
  onPatchDraft,
  onCancel,
  onConfirm,
  onContinuePartial
}: {
  open: boolean
  pending: boolean
  partialSetup: ReturnType<typeof useStudio>['goLivePartialSetup']
  preflight: ReturnType<typeof useStudio>['goLivePreflight']
  draft: ReturnType<typeof useStudio>['streamMetadataDraft']
  onPatchDraft: ReturnType<typeof useStudio>['patchStreamMetadataDraft']
  onCancel: () => void
  onConfirm: () => void
  onContinuePartial: () => void
}): ReactElement {
  const errorCount = preflight?.issues.filter((issue) => issue.severity === 'error').length ?? 0

  return (
    <Dialog open={open} onOpenChange={(next) => !next && onCancel()}>
      <DialogContent className="max-h-[88vh] gap-4 overflow-hidden sm:max-w-3xl">
        <DialogHeader>
          <DialogTitle>Confirm Go Live</DialogTitle>
          <DialogDescription>
            Review destinations and metadata before Videogre starts the livestream.
          </DialogDescription>
        </DialogHeader>

        <ScrollArea className="max-h-[60vh] pr-3">
          <div className="flex flex-col gap-4">
            <div className="grid gap-3 sm:grid-cols-2">
              <Field>
                <FieldLabel htmlFor="go-live-title">Title</FieldLabel>
                <Input
                  id="go-live-title"
                  disabled={pending || !draft}
                  value={draft?.title ?? ''}
                  onChange={(event) => onPatchDraft({ title: event.target.value })}
                />
              </Field>
              <Field>
                <FieldLabel htmlFor="go-live-description">Description</FieldLabel>
                <textarea
                  className="min-h-20 rounded-md border bg-background px-3 py-2 text-sm outline-none transition focus-visible:border-ring focus-visible:ring-[3px] focus-visible:ring-ring/30 disabled:cursor-not-allowed disabled:opacity-50"
                  disabled={pending || !draft}
                  id="go-live-description"
                  value={draft?.description ?? ''}
                  onChange={(event) => onPatchDraft({ description: event.target.value })}
                />
              </Field>
            </div>

            <div className="flex flex-col gap-2">
              <div className="flex items-center justify-between gap-3">
                <span className="text-sm font-medium">Destinations</span>
                {errorCount ? (
                  <Badge variant="destructive">{errorCount} issue{errorCount === 1 ? '' : 's'}</Badge>
                ) : (
                  <Badge variant="success">Ready</Badge>
                )}
              </div>
              <div className="grid gap-2">
                {preflight?.destinations.length ? (
                  preflight.destinations.map((destination) => (
                    <GoLiveDestinationRow destination={destination} key={destination.targetId} />
                  ))
                ) : (
                  <div className="rounded-md border border-dashed p-3 text-sm text-muted-foreground">
                    No livestream destinations are enabled.
                  </div>
                )}
              </div>
            </div>

            {preflight?.issues.length ? (
              <div className="flex flex-col gap-2 rounded-md border border-destructive/25 bg-destructive/5 p-3">
                <div className="flex items-center gap-2 text-sm font-medium text-destructive">
                  <WarningCircle className="size-4" weight="fill" />
                  Resolve before going live
                </div>
                <ul className="grid gap-1.5 text-sm text-muted-foreground">
                  {preflight.issues.map((issue, index) => (
                    <li key={`${issue.platform ?? 'global'}-${issue.targetId ?? 'all'}-${index}`}>
                      {issue.platform ? `${platformLabel(issue.platform)}: ` : ''}
                      {issue.message}
                    </li>
                  ))}
                </ul>
              </div>
            ) : null}

            {partialSetup ? (
              <div className="flex flex-col gap-2 rounded-md border border-warning/35 bg-warning/10 p-3">
                <div className="flex items-center gap-2 text-sm font-medium">
                  <WarningCircle className="size-4 text-warning" weight="fill" />
                  Some destinations failed setup
                </div>
                <ul className="grid gap-1.5 text-sm text-muted-foreground">
                  {partialSetup.failures.map((failure) => (
                    <li key={failure.targetId}>
                      {platformLabel(failure.platform)}: {failure.label} - {failure.message}
                    </li>
                  ))}
                </ul>
                <p className="text-xs text-muted-foreground">
                  Ready: {partialSetup.readyLabels.join(', ')}
                </p>
              </div>
            ) : null}
          </div>
        </ScrollArea>

        <DialogFooter>
          <Button disabled={pending} variant="outline" onClick={onCancel}>
            Cancel
          </Button>
          {partialSetup ? (
            <Button disabled={pending} onClick={onContinuePartial}>
              <Broadcast data-icon="inline-start" weight="fill" />
              {pending ? 'Starting…' : 'Continue With Ready'}
            </Button>
          ) : (
            <Button disabled={pending || !preflight} onClick={onConfirm}>
              <Broadcast data-icon="inline-start" weight="fill" />
              {pending ? 'Checking…' : 'Confirm Go Live'}
            </Button>
          )}
        </DialogFooter>
      </DialogContent>
    </Dialog>
  )
}

function GoLiveDestinationRow({
  destination
}: {
  destination: GoLiveDestinationPreflight
}): ReactElement {
  return (
    <div className="grid gap-2 rounded-md border bg-muted/25 p-3 sm:grid-cols-[1fr_auto]">
      <div className="min-w-0">
        <div className="flex flex-wrap items-center gap-2">
          <span className="font-medium">{destination.label}</span>
          <Badge variant={destination.ready ? 'success' : 'destructive'}>
            {destination.ready ? (
              <CheckCircle data-icon="inline-start" weight="fill" />
            ) : (
              <WarningCircle data-icon="inline-start" weight="fill" />
            )}
            {destination.ready ? 'Ready' : 'Blocked'}
          </Badge>
          <Badge variant="outline">{destination.authMode === 'oauth' ? 'OAuth' : 'Manual RTMP'}</Badge>
        </div>
        <p className="mt-1 truncate text-sm text-muted-foreground">{destination.title || 'Untitled'}</p>
        {destination.accountLabel ? (
          <p className="mt-0.5 text-xs text-muted-foreground">{destination.accountLabel}</p>
        ) : null}
      </div>
      <p className="text-xs text-muted-foreground sm:max-w-64 sm:text-right">{destination.message}</p>
    </div>
  )
}

function platformLabel(platform: StreamPlatform): string {
  switch (platform) {
    case 'youtube':
      return 'YouTube'
    case 'twitch':
      return 'Twitch'
    case 'x':
      return 'X'
    case 'custom':
      return 'Custom RTMP'
  }
}

function MixerRow({
  selectedMicrophoneName,
  meterLevel,
  gainDb,
  muted,
  peakDb,
  syncOffsetMs
}: {
  selectedMicrophoneName?: string
  meterLevel: number
  gainDb: number
  muted: boolean
  peakDb?: number
  syncOffsetMs: number
}): ReactElement {
  const meterTone = muted ? 'bg-muted-foreground/30' : meterLevel > 2 ? 'bg-success' : 'bg-warning'

  return (
    <div className="grid gap-2">
      <div className="flex items-center justify-between gap-3 text-sm">
        <span className="min-w-0 truncate text-muted-foreground">
          {selectedMicrophoneName ?? 'No microphone selected'}
        </span>
        <span className="font-mono text-xs tabular-nums text-muted-foreground">
          {muted ? 'Muted' : `${gainDb > 0 ? '+' : ''}${gainDb} dB`}
        </span>
      </div>
      <div className="h-2.5 w-full overflow-hidden rounded-full bg-muted">
        <div
          className={cn('h-full rounded-full transition-all', meterTone)}
          style={{ width: `${Math.min(100, Math.max(0, meterLevel))}%` }}
        />
      </div>
      <div className="flex items-center justify-between text-xs text-muted-foreground">
        <span>Source meter</span>
        <span>{typeof peakDb === 'number' ? `${peakDb.toFixed(1)} dB` : 'Not checked'}</span>
      </div>
      <div className="flex items-center justify-between text-xs text-muted-foreground">
        <span>Sync</span>
        <span>{`${syncOffsetMs > 0 ? '+' : ''}${syncOffsetMs} ms`}</span>
      </div>
    </div>
  )
}

function StudioScreensRow({
  screens,
  activeScreen,
  onActivate,
  onClear,
  onOpenScreens
}: {
  screens: StreamScreen[]
  activeScreen: StreamScreen | null
  onActivate: (screenId: string) => void
  onClear: () => void
  onOpenScreens: () => void
}): ReactElement {
  if (screens.length === 0) {
    return (
      <div className="flex items-center justify-between gap-3 rounded-lg border bg-muted/30 px-3 py-2">
        <span className="min-w-0 truncate text-sm text-muted-foreground">No Screens uploaded</span>
        <Button size="sm" variant="secondary" onClick={onOpenScreens}>
          <ImageSquare data-icon="inline-start" weight="duotone" />
          Add
        </Button>
      </div>
    )
  }

  return (
    <div className="flex flex-wrap gap-2">
      <Button
        size="sm"
        variant={activeScreen ? 'outline' : 'default'}
        onClick={activeScreen ? onClear : undefined}
      >
        Normal
      </Button>
      {screens.map((screen) => {
        const selected = activeScreen?.id === screen.id
        const missing = screen.status === 'missing'
        return (
          <Button
            disabled={missing}
            key={screen.id}
            size="sm"
            title={missing ? 'Screen image is missing' : screen.name}
            variant={selected ? 'default' : 'outline'}
            onClick={() => (selected ? onClear() : onActivate(screen.id))}
          >
            <span className="flex min-w-0 items-center gap-2">
              <span className="h-5 w-8 shrink-0 overflow-hidden rounded border bg-muted">
                {!missing ? (
                  <img alt="" className="size-full object-cover" src={fileUrlFromPath(screen.imagePath)} />
                ) : (
                  <span className="block size-full bg-destructive/20" />
                )}
              </span>
              <span className="max-w-32 truncate">{screen.name}</span>
            </span>
          </Button>
        )
      })}
    </div>
  )
}

function SummaryRow({ label, value }: { label: string; value: string }): ReactElement {
  return (
    <>
      <dt className="text-muted-foreground">{label}</dt>
      <dd className="truncate text-right font-medium">{value}</dd>
    </>
  )
}

function fileUrlFromPath(path: string): string {
  const normalized = path.replace(/\\/g, '/')
  const prefix = /^[A-Za-z]:/.test(normalized) ? 'file:///' : 'file://'
  return `${prefix}${encodeURI(normalized)}`
}

function pipelineStatusLabel(status: string): string {
  switch (status) {
    case 'finalizing':
      return 'Finalizing'
    case 'finalized':
      return 'Finalized'
    case 'failed':
      return 'Failed'
    default:
      return 'Running'
  }
}

function studioBlocker(studio: ReturnType<typeof useStudio>): {
  title: string
  jumpTo?: WorkspaceTab | StudioPanel
  jumpLabel?: string
} | null {
  const { wsStatus, outputEnabled, captureConfig, streamReady, health } = studio

  if (wsStatus !== 'connected') {
    return { title: 'Backend not connected' }
  }
  if (!outputEnabled) {
    return { title: 'No output enabled', jumpTo: 'recording', jumpLabel: 'Open Recording' }
  }
  if (captureConfig.streamEnabled && !streamReady) {
    return { title: 'Stream target incomplete', jumpTo: 'live', jumpLabel: 'Open Live' }
  }
  if (health && !health.ffmpeg.available) {
    return { title: 'FFmpeg unavailable', jumpTo: 'settings', jumpLabel: 'Open Settings' }
  }
  return { title: 'Finish setup to start' }
}
