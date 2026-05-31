import { ArrowsClockwise, FolderOpen, GearSix, Image, PencilSimpleLine, VideoCamera } from '@phosphor-icons/react'
import { useEffect, useMemo, useState, type CSSProperties, type ReactElement } from 'react'

import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import type { LayoutSettings, PreviewLiveStatus, RuntimeInfo, Scene } from '@/lib/backend'
import { cn } from '@/lib/utils'

const SIZE_FRACTION: Record<LayoutSettings['cameraSize'], string> = {
  small: '20%',
  medium: '26%',
  large: '34%'
}

function cameraBoxStyle(layout: LayoutSettings): CSSProperties {
  const margin = `${layout.cameraMargin / 16}rem`
  const style: CSSProperties = {
    width: SIZE_FRACTION[layout.cameraSize],
    aspectRatio: '16 / 9',
    position: 'absolute'
  }

  if (layout.cameraCorner.includes('top')) {
    style.top = margin
  } else {
    style.bottom = margin
  }
  if (layout.cameraCorner.includes('left')) {
    style.left = margin
  } else {
    style.right = margin
  }

  return style
}

export function PreviewStage({
  previewUrl,
  previewLoading,
  previewLiveStatus,
  layout,
  onRetry,
  onOpenPermissions,
  onRevealPermissionTarget,
  runtimeInfo,
  scene,
  sceneEditMode = false,
  selectedSceneSourceId,
  onSelectSceneSource,
  className
}: {
  previewUrl: string | null
  previewLoading: boolean
  previewLiveStatus: PreviewLiveStatus
  layout: LayoutSettings
  onRetry?: () => void
  onOpenPermissions?: () => void
  onRevealPermissionTarget?: () => void
  runtimeInfo?: RuntimeInfo | null
  scene?: Scene | null
  sceneEditMode?: boolean
  selectedSceneSourceId?: string | null
  onSelectSceneSource?: (sourceId: string) => void
  className?: string
}): ReactElement {
  const [imageFailed, setImageFailed] = useState(false)
  const [displayPreviewUrl, setDisplayPreviewUrl] = useState<string | null>(previewUrl)
  const isLive = previewLiveStatus.state === 'live'
  const latestFrameUrl = useMemo(() => latestPreviewFrameUrl(previewUrl), [previewUrl])
  const showUnavailable = previewLiveStatus.state === 'unavailable' || imageFailed
  const badgeLabel =
    previewLiveStatus.state === 'connecting'
      ? 'Connecting'
      : previewLiveStatus.state === 'reconnecting'
        ? 'Reconnecting'
        : isLive
          ? previewLiveStatus.source === 'recording-session'
            ? 'Recording live'
            : 'Live'
          : 'Unavailable'

  useEffect(() => {
    setImageFailed(false)
  }, [previewUrl])

  useEffect(() => {
    if (!previewUrl) {
      setDisplayPreviewUrl(null)
      return
    }

    if (!isLive || !latestFrameUrl) {
      setDisplayPreviewUrl(previewUrl)
      return
    }

    const updateFrame = (): void => {
      setDisplayPreviewUrl(withCacheBust(latestFrameUrl))
    }

    updateFrame()
    const timer = window.setInterval(updateFrame, 80)
    return () => window.clearInterval(timer)
  }, [isLive, latestFrameUrl, previewUrl])

  return (
    <div className={cn('flex flex-col gap-3', className)}>
      <div className="relative aspect-video w-full overflow-hidden rounded-xl border bg-muted">
        {displayPreviewUrl && !imageFailed ? (
          <img
            alt="Selected scene preview"
            className="size-full object-contain"
            src={displayPreviewUrl}
            onLoad={() => setImageFailed(false)}
            onError={() => {
              if (!isLive || !latestFrameUrl) {
                setImageFailed(true)
              }
            }}
          />
        ) : (
          <div className="flex size-full items-center justify-center">
            {showUnavailable ? (
              <div className="flex max-w-sm flex-col items-center gap-2 px-6 text-center">
                <Image className="size-10 text-muted-foreground/50" weight="duotone" />
                <p className="text-sm font-medium text-muted-foreground">
                  {imageFailed
                    ? 'Live preview stream could not be displayed.'
                    : (previewLiveStatus.message ?? 'Live preview unavailable.')}
                </p>
              </div>
            ) : (
              <VideoCamera className="size-10 text-muted-foreground/50" weight="duotone" />
            )}
            {!showUnavailable ? (
              <div
                className={cn(
                  'border-2 border-primary/60 bg-primary/10',
                  layout.cameraShape === 'circle' ? 'rounded-full' : 'rounded-md'
                )}
                style={cameraBoxStyle(layout)}
              />
            ) : null}
          </div>
        )}
        <Badge className="absolute top-2 left-2" variant={isLive ? 'success' : 'secondary'}>
          {previewLoading ? 'Connecting' : badgeLabel}
        </Badge>
        {sceneEditMode ? (
          <Badge className="absolute top-2 right-2" variant="warning">
            <PencilSimpleLine data-icon="inline-start" />
            Edit
          </Badge>
        ) : null}
        {sceneEditMode && scene ? (
          <div className="pointer-events-none absolute inset-0">
            {scene.sources
              .filter((source) => source.visible)
              .map((source) => {
                const selected = source.id === selectedSceneSourceId
                return (
                  <button
                    aria-label={`Select ${source.name}`}
                    className={cn(
                      'pointer-events-auto absolute border text-left transition-colors',
                      'appearance-none p-0',
                      selected
                        ? 'border-primary bg-primary/10 ring-2 ring-primary/40'
                        : 'border-primary/50 bg-primary/5 hover:bg-primary/10'
                    )}
                    key={source.id}
                    style={sceneSourceStyle(source.transform)}
                    type="button"
                    onClick={() => onSelectSceneSource?.(source.id)}
                  >
                    <span className="absolute -top-6 left-0 rounded-md bg-background/95 px-1.5 py-0.5 text-[10px] font-medium text-foreground shadow-sm">
                      {source.name}
                    </span>
                    {selected ? <TransformHandles /> : null}
                  </button>
                )
              })}
          </div>
        ) : null}
      </div>
      {showUnavailable && (onRetry || onOpenPermissions) ? (
        <div className="flex flex-col gap-2">
          <div className="flex flex-wrap gap-2">
            {onOpenPermissions ? (
              <Button size="sm" variant="outline" onClick={onOpenPermissions}>
                <GearSix data-icon="inline-start" />
                Open permissions
              </Button>
            ) : null}
            {runtimeInfo && !runtimeInfo.isPackaged && onRevealPermissionTarget ? (
              <Button size="sm" variant="outline" onClick={onRevealPermissionTarget}>
                <FolderOpen data-icon="inline-start" />
                Reveal Electron.app
              </Button>
            ) : null}
            {onRetry ? (
              <Button size="sm" variant="outline" onClick={onRetry}>
                <ArrowsClockwise data-icon="inline-start" />
                Retry preview
              </Button>
            ) : null}
          </div>
          {runtimeInfo && !runtimeInfo.isPackaged ? (
            <p className="text-xs text-muted-foreground">
              Dev mode needs Screen Recording permission for {runtimeInfo.permissionTargetName}. If it is not listed,
              add the revealed app manually, then relaunch.
            </p>
          ) : null}
        </div>
      ) : null}
    </div>
  )
}

function latestPreviewFrameUrl(previewUrl: string | null): string | null {
  if (!previewUrl) {
    return null
  }

  return previewUrl.includes('/preview/live.mjpeg')
    ? previewUrl.replace('/preview/live.mjpeg', '/preview/live.jpg')
    : null
}

function withCacheBust(url: string): string {
  const separator = url.includes('?') ? '&' : '?'
  return `${url}${separator}t=${Date.now()}`
}

function sceneSourceStyle(transform: Scene['sources'][number]['transform']): CSSProperties {
  return {
    left: `${transform.x * 100}%`,
    top: `${transform.y * 100}%`,
    width: `${transform.width * 100}%`,
    height: `${transform.height * 100}%`
  }
}

function TransformHandles(): ReactElement {
  return (
    <>
      {[
        'top-0 left-0 -translate-x-1/2 -translate-y-1/2',
        'top-0 left-1/2 -translate-x-1/2 -translate-y-1/2',
        'top-0 right-0 translate-x-1/2 -translate-y-1/2',
        'top-1/2 right-0 translate-x-1/2 -translate-y-1/2',
        'right-0 bottom-0 translate-x-1/2 translate-y-1/2',
        'bottom-0 left-1/2 -translate-x-1/2 translate-y-1/2',
        'bottom-0 left-0 -translate-x-1/2 translate-y-1/2',
        'top-1/2 left-0 -translate-x-1/2 -translate-y-1/2'
      ].map((position) => (
        <span className={cn('absolute size-2 rounded-sm border border-background bg-primary', position)} key={position} />
      ))}
    </>
  )
}
