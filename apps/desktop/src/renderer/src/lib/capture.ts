import type {
  AudioSettings,
  CameraTransform,
  CameraTransformMode,
  LayoutPreset,
  LayoutSettings,
  RtmpPreset,
  SideBySideCameraSide,
  SideBySideSplit,
  SourceSelection,
  StreamingSettings,
  StreamPlatform,
  StreamTargetSettings,
  VideoPreset,
  VideoSettings
} from '../../../shared/backend'

export type SettingsState = {
  outputDirectory: string
  ffmpegPath: string
}

export type CaptureConfig = {
  sources: SourceSelection
  layout: LayoutSettings
  audio: AudioSettings
  video: VideoSettings
  recordEnabled: boolean
  streamEnabled: boolean
  rtmpPreset: RtmpPreset
  rtmpServerUrl: string
  streamKey: string
  streaming: StreamingSettings
}

export type WsStatus = 'waiting' | 'connecting' | 'connected' | 'failed' | 'closed'
export type SetupTone = 'good' | 'warn' | 'neutral'
export type SetupStep = {
  label: string
  detail: string
  tone: SetupTone
}

export const STORAGE_KEYS = {
  settings: 'videorc.settings',
  captureConfig: 'videorc.captureConfig',
  onboarding: 'videorc.onboardingComplete',
  theme: 'videorc.theme'
} as const

export const ONBOARDING_VERSION = 'creator-ux-v1'

export const defaultSettings: SettingsState = {
  outputDirectory: '',
  ffmpegPath: ''
}

export const rtmpDefaults: Record<RtmpPreset, string> = {
  youtube: 'rtmp://a.rtmp.youtube.com/live2',
  twitch: 'rtmp://live.twitch.tv/app',
  x: '',
  custom: ''
}

// Fixed destination order for the Streaming tab (YouTube, Twitch, X, Custom).
export const STREAM_PLATFORM_ORDER: readonly StreamPlatform[] = ['youtube', 'twitch', 'x', 'custom']

const STREAM_PLATFORM_LABELS: Record<StreamPlatform, string> = {
  youtube: 'YouTube',
  twitch: 'Twitch',
  x: 'X / Twitter',
  custom: 'Custom RTMP'
}

function isStreamPlatform(value: unknown): value is StreamPlatform {
  return typeof value === 'string' && (STREAM_PLATFORM_ORDER as readonly string[]).includes(value)
}

function makeStreamTarget(platform: StreamPlatform, now: string): StreamTargetSettings {
  return {
    // One built-in target per platform in v1, so the platform name is a stable id.
    id: platform,
    platform,
    label: STREAM_PLATFORM_LABELS[platform],
    enabled: false,
    serverUrl: rtmpDefaults[platform],
    urlMode: 'server-and-key',
    streamKey: '',
    streamKeyPresent: false,
    authMode: 'manual-rtmp',
    status: { state: 'not-configured' },
    createdAt: now,
    updatedAt: now
  }
}

function defaultStreamTargets(now: string = new Date().toISOString()): StreamTargetSettings[] {
  return STREAM_PLATFORM_ORDER.map((platform) => makeStreamTarget(platform, now))
}

export function defaultStreamingSettings(): StreamingSettings {
  return {
    enabled: false,
    mode: 'single',
    targets: defaultStreamTargets(),
    defaultOutputPreset: 'tutorial-1080p30',
    defaultBitrateKbps: 6000,
    enabledTargetIds: []
  }
}

export const videoPresets: Record<VideoPreset, VideoSettings> = {
  'tutorial-1080p30': {
    preset: 'tutorial-1080p30',
    width: 1920,
    height: 1080,
    fps: 30,
    bitrateKbps: 6000
  },
  'tutorial-1440p30': {
    preset: 'tutorial-1440p30',
    width: 2560,
    height: 1440,
    fps: 30,
    bitrateKbps: 8000
  },
  'stream-1080p60': {
    preset: 'stream-1080p60',
    width: 1920,
    height: 1080,
    fps: 60,
    bitrateKbps: 9000
  },
  custom: {
    preset: 'custom',
    width: 1920,
    height: 1080,
    fps: 30,
    bitrateKbps: 6000
  }
}

export const defaultCaptureConfig: CaptureConfig = {
  sources: {},
  layout: {
    layoutPreset: 'screen-camera',
    cameraTransformMode: 'preset',
    cameraTransform: null,
    cameraCorner: 'bottom-right',
    cameraSize: 'medium',
    cameraShape: 'rectangle',
    cameraMargin: 32,
    cameraFit: 'fill',
    cameraMirror: false,
    cameraZoom: 100,
    cameraOffsetX: 0,
    cameraOffsetY: 0,
    sideBySideSplit: '70-30',
    sideBySideCameraSide: 'right'
  },
  audio: {
    microphoneGainDb: 0,
    microphoneMuted: false,
    microphoneSyncOffsetMs: -250
  },
  video: videoPresets['tutorial-1440p30'],
  recordEnabled: true,
  streamEnabled: false,
  rtmpPreset: 'youtube',
  rtmpServerUrl: rtmpDefaults.youtube,
  streamKey: '',
  streaming: defaultStreamingSettings()
}

export function loadJson<T>(key: string, fallback: T): T {
  const raw = localStorage.getItem(key)
  if (!raw) {
    return fallback
  }

  try {
    return { ...fallback, ...(JSON.parse(raw) as Partial<T>) }
  } catch {
    return fallback
  }
}

export function loadCaptureConfig(): CaptureConfig {
  const loaded = loadJson(STORAGE_KEYS.captureConfig, defaultCaptureConfig) as Partial<CaptureConfig>

  return {
    ...defaultCaptureConfig,
    ...loaded,
    sources: { ...defaultCaptureConfig.sources, ...(loaded.sources ?? {}) },
    layout: normalizeLayoutSettings(loaded.layout),
    audio: normalizeAudioSettings(loaded.audio),
    video: normalizeVideoSettings(loaded.video),
    recordEnabled:
      typeof loaded.recordEnabled === 'boolean' ? loaded.recordEnabled : defaultCaptureConfig.recordEnabled,
    streamEnabled:
      typeof loaded.streamEnabled === 'boolean' ? loaded.streamEnabled : defaultCaptureConfig.streamEnabled,
    rtmpPreset: loaded.rtmpPreset ?? defaultCaptureConfig.rtmpPreset,
    rtmpServerUrl: loaded.rtmpServerUrl ?? defaultCaptureConfig.rtmpServerUrl,
    streamKey: loaded.streamKey ?? defaultCaptureConfig.streamKey,
    streaming: migrateStreamingSettings(loaded)
  }
}

export function normalizeAudioSettings(audio: unknown): AudioSettings {
  const candidate = audio && typeof audio === 'object' ? (audio as Partial<AudioSettings>) : {}

  return {
    microphoneGainDb: clampNumber(candidate.microphoneGainDb, defaultCaptureConfig.audio.microphoneGainDb, -24, 24),
    microphoneMuted:
      typeof candidate.microphoneMuted === 'boolean'
        ? candidate.microphoneMuted
        : defaultCaptureConfig.audio.microphoneMuted,
    microphoneSyncOffsetMs: clampNumber(
      candidate.microphoneSyncOffsetMs,
      defaultCaptureConfig.audio.microphoneSyncOffsetMs,
      -1000,
      1000
    )
  }
}

const LAYOUT_PRESET_VALUES: readonly LayoutPreset[] = [
  'screen-camera',
  'screen-only',
  'camera-only',
  'side-by-side'
]

function isLayoutPreset(value: unknown): value is LayoutPreset {
  return typeof value === 'string' && (LAYOUT_PRESET_VALUES as readonly string[]).includes(value)
}

const SIDE_BY_SIDE_SPLITS: readonly SideBySideSplit[] = ['50-50', '60-40', '70-30']

function isSideBySideSplit(value: unknown): value is SideBySideSplit {
  return typeof value === 'string' && (SIDE_BY_SIDE_SPLITS as readonly string[]).includes(value)
}

function clampUnit(value: number): number {
  return Math.min(1, Math.max(0, value))
}

function normalizeCameraTransform(value: unknown): CameraTransform | null {
  if (!value || typeof value !== 'object') {
    return null
  }

  const candidate = value as Partial<CameraTransform>
  const x = Number(candidate.x)
  const y = Number(candidate.y)
  const width = Number(candidate.width)
  const height = Number(candidate.height)
  if (![x, y, width, height].every((entry) => Number.isFinite(entry)) || width <= 0 || height <= 0) {
    return null
  }

  return { x: clampUnit(x), y: clampUnit(y), width: clampUnit(width), height: clampUnit(height) }
}

export function normalizeLayoutSettings(layout: unknown): LayoutSettings {
  const candidate = layout && typeof layout === 'object' ? (layout as Partial<LayoutSettings>) : {}
  const cameraTransform = normalizeCameraTransform(candidate.cameraTransform)
  const cameraTransformMode: CameraTransformMode =
    candidate.cameraTransformMode === 'custom' && cameraTransform ? 'custom' : 'preset'

  return {
    ...defaultCaptureConfig.layout,
    ...candidate,
    layoutPreset: isLayoutPreset(candidate.layoutPreset)
      ? candidate.layoutPreset
      : defaultCaptureConfig.layout.layoutPreset,
    cameraTransformMode,
    cameraTransform: cameraTransformMode === 'custom' ? cameraTransform : null,
    cameraMargin: clampNumber(candidate.cameraMargin, defaultCaptureConfig.layout.cameraMargin, 8, 96),
    cameraZoom: clampNumber(candidate.cameraZoom, defaultCaptureConfig.layout.cameraZoom, 100, 200),
    cameraOffsetX: clampNumber(candidate.cameraOffsetX, defaultCaptureConfig.layout.cameraOffsetX, -100, 100),
    cameraOffsetY: clampNumber(candidate.cameraOffsetY, defaultCaptureConfig.layout.cameraOffsetY, -100, 100),
    cameraMirror:
      typeof candidate.cameraMirror === 'boolean' ? candidate.cameraMirror : defaultCaptureConfig.layout.cameraMirror,
    cameraFit:
      candidate.cameraFit === 'fit' || candidate.cameraFit === 'fill'
        ? candidate.cameraFit
        : defaultCaptureConfig.layout.cameraFit,
    sideBySideSplit: isSideBySideSplit(candidate.sideBySideSplit)
      ? candidate.sideBySideSplit
      : defaultCaptureConfig.layout.sideBySideSplit,
    sideBySideCameraSide:
      candidate.sideBySideCameraSide === 'left' || candidate.sideBySideCameraSide === 'right'
        ? candidate.sideBySideCameraSide
        : defaultCaptureConfig.layout.sideBySideCameraSide
  }
}

export function normalizeVideoSettings(video: unknown): VideoSettings {
  const candidate = video && typeof video === 'object' ? (video as Partial<VideoSettings>) : {}
  const preset =
    typeof candidate.preset === 'string' && candidate.preset in videoPresets
      ? (candidate.preset as VideoPreset)
      : defaultCaptureConfig.video.preset
  const fallback = videoPresets[preset]

  return {
    preset,
    width: clampNumber(candidate.width, fallback.width, 640, 3840),
    height: clampNumber(candidate.height, fallback.height, 360, 2160),
    fps: clampNumber(candidate.fps, fallback.fps, 24, 60),
    bitrateKbps: clampNumber(candidate.bitrateKbps, fallback.bitrateKbps, 1000, 50000)
  }
}

function normalizeStreamTarget(
  base: StreamTargetSettings,
  saved: Partial<StreamTargetSettings>
): StreamTargetSettings {
  const streamKey = typeof saved.streamKey === 'string' ? saved.streamKey : ''
  const serverUrl =
    typeof saved.serverUrl === 'string' && saved.serverUrl.trim() ? saved.serverUrl : base.serverUrl
  return {
    // id, platform, authMode, status keep the built-in identity from base.
    ...base,
    label: typeof saved.label === 'string' && saved.label ? saved.label : base.label,
    enabled: typeof saved.enabled === 'boolean' ? saved.enabled : false,
    serverUrl,
    urlMode: saved.urlMode === 'full-url' ? 'full-url' : 'server-and-key',
    streamKey,
    streamKeySecretRef: typeof saved.streamKeySecretRef === 'string' ? saved.streamKeySecretRef : undefined,
    streamKeyPresent: streamKey.length > 0,
    createdAt: typeof saved.createdAt === 'string' ? saved.createdAt : base.createdAt,
    updatedAt: typeof saved.updatedAt === 'string' ? saved.updatedAt : base.updatedAt
  }
}

export function normalizeStreamingSettings(value: unknown): StreamingSettings {
  const candidate = (value && typeof value === 'object' ? value : {}) as Partial<StreamingSettings>
  const now = new Date().toISOString()
  const persisted = Array.isArray(candidate.targets) ? candidate.targets : []
  const targets = STREAM_PLATFORM_ORDER.map((platform) => {
    const base = makeStreamTarget(platform, now)
    const saved = persisted.find(
      (target) =>
        target && typeof target === 'object' && (target.id === platform || target.platform === platform)
    )
    return saved ? normalizeStreamTarget(base, saved) : base
  })
  const enabledTargetIds = targets.filter((target) => target.enabled).map((target) => target.id)
  const defaultOutputPreset: VideoPreset =
    typeof candidate.defaultOutputPreset === 'string' && candidate.defaultOutputPreset in videoPresets
      ? (candidate.defaultOutputPreset as VideoPreset)
      : 'tutorial-1080p30'

  return {
    enabled: typeof candidate.enabled === 'boolean' ? candidate.enabled : enabledTargetIds.length > 0,
    mode: enabledTargetIds.length > 1 ? 'multi' : 'single',
    targets,
    selectedTargetId: typeof candidate.selectedTargetId === 'string' ? candidate.selectedTargetId : undefined,
    defaultOutputPreset,
    defaultBitrateKbps: clampNumber(candidate.defaultBitrateKbps, 6000, 1000, 50000),
    enabledTargetIds
  }
}

export function migrateStreamingSettings(loaded: Partial<CaptureConfig>): StreamingSettings {
  // Already on the per-target model: normalize and return.
  if (loaded.streaming && typeof loaded.streaming === 'object') {
    return normalizeStreamingSettings(loaded.streaming)
  }

  // Legacy single-RTMP config: move the server/key into the matching platform
  // target only, so the other platforms keep their own (empty) credentials and
  // a YouTube key can never overwrite a Twitch/X key.
  const now = new Date().toISOString()
  const targets = defaultStreamTargets(now)
  const legacyPlatform: StreamPlatform = isStreamPlatform(loaded.rtmpPreset) ? loaded.rtmpPreset : 'youtube'
  const legacyKey = typeof loaded.streamKey === 'string' ? loaded.streamKey : ''
  const legacyServer = typeof loaded.rtmpServerUrl === 'string' ? loaded.rtmpServerUrl.trim() : ''
  const enabled = typeof loaded.streamEnabled === 'boolean' ? loaded.streamEnabled : false
  const match = targets.find((target) => target.platform === legacyPlatform)
  if (match) {
    if (legacyServer) {
      match.serverUrl = legacyServer
    }
    match.streamKey = legacyKey
    match.streamKeyPresent = legacyKey.length > 0
    match.enabled = enabled
    match.updatedAt = now
  }

  return {
    enabled,
    mode: 'single',
    targets,
    defaultOutputPreset: 'tutorial-1080p30',
    defaultBitrateKbps: 6000,
    enabledTargetIds: match && match.enabled ? [match.id] : []
  }
}

export function clampNumber(value: unknown, fallback: number, min: number, max: number): number {
  const parsed = Number(value)
  if (!Number.isFinite(parsed)) {
    return fallback
  }

  return Math.min(max, Math.max(min, Math.round(parsed)))
}
