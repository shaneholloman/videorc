export interface BackendConnection {
  host: string
  port: number
  token: string
}

export interface BackendHealth {
  status: string
  version: string
  platform: string
  ffmpeg: ToolStatus
  databasePath: string
}

export interface ToolStatus {
  path: string
  available: boolean
  version?: string
  message?: string
}

export type DeviceKind = 'screen' | 'window' | 'camera' | 'microphone' | 'system-audio'
export type DeviceStatus = 'available' | 'unavailable' | 'permission-required'

export interface Device {
  id: string
  name: string
  kind: DeviceKind
  status: DeviceStatus
  detail?: string
}

export interface DeviceList {
  devices: Device[]
  warnings: string[]
}

export type RecordingState = 'idle' | 'starting' | 'recording' | 'streaming' | 'stopping' | 'failed'

export interface RecordingStatus {
  state: RecordingState
  sessionId?: string
  outputPath?: string
  streamUrl?: string
  startedAt?: string
  message?: string
}

export interface BackendLogEvent {
  level: 'info' | 'warn' | 'error' | string
  message: string
  timestamp: string
}

export interface ClientCommand<TParams = unknown> {
  id: string
  method: string
  params?: TParams
}

export interface ServerResponse<TPayload = unknown> {
  id: string
  ok: boolean
  payload?: TPayload
  error?: {
    code: string
    message: string
  }
}

export interface ServerEvent<TPayload = unknown> {
  event: string
  payload: TPayload
}

export interface StartRecordingParams {
  outputDirectory?: string
  ffmpegPath?: string
}

export interface SourceSelection {
  screenId?: string
  windowId?: string
  cameraId?: string
  microphoneId?: string
}

export type CameraCorner = 'top-left' | 'top-right' | 'bottom-left' | 'bottom-right'
export type CameraSize = 'small' | 'medium' | 'large'
export type CameraShape = 'rectangle' | 'circle'

export interface LayoutSettings {
  cameraCorner: CameraCorner
  cameraSize: CameraSize
  cameraShape: CameraShape
  cameraMargin: number
}

export type RtmpPreset = 'youtube' | 'twitch' | 'x' | 'custom'

export interface RtmpSettings {
  preset: RtmpPreset
  serverUrl: string
  streamKey: string
}

export interface OutputSettings {
  recordEnabled: boolean
  streamEnabled: boolean
  outputDirectory?: string
  ffmpegPath?: string
  rtmp: RtmpSettings
}

export interface StartSessionParams {
  sources: SourceSelection
  layout: LayoutSettings
  output: OutputSettings
}

export interface RemuxSessionParams {
  sessionId: string
  ffmpegPath?: string
}

export interface PreviewSnapshotParams {
  sources: SourceSelection
  layout: LayoutSettings
  ffmpegPath?: string
}

export interface PreviewSnapshot {
  id: string
  url: string
  createdAt: string
}

export type HealthLevel = 'info' | 'warn' | 'error'

export interface HealthEvent {
  id: string
  sessionId?: string
  level: HealthLevel
  code: string
  message: string
  createdAt: string
}

export interface RunAiWorkflowParams {
  sessionId: string
  consentToUploadAudio: boolean
  ffmpegPath?: string
}

export interface AiWorkflowResult {
  sessionId: string
  audioPath: string
  artifacts: AiArtifact[]
}

export type AiArtifactKind = 'audio-extract' | 'transcript' | 'summary' | 'chapters'
export type AiArtifactStatus = 'ready' | 'pending-consent' | 'failed'

export interface AiArtifact {
  id: string
  sessionId: string
  kind: AiArtifactKind
  status: AiArtifactStatus
  content: unknown
  filePath?: string
  createdAt: string
}

export interface SessionSummary {
  id: string
  title: string
  startedAt: string
  endedAt?: string
  status: string
  mode: string
  outputPath?: string
  mp4Path?: string
  streamPreset?: string
  layout: LayoutSettings
  sources: SourceSelection
  healthEvents: HealthEvent[]
  aiArtifacts: AiArtifact[]
}

export interface VideogreApi {
  getBackendConnection: () => Promise<BackendConnection | null>
  getBackendLogs: () => Promise<BackendLogEvent[]>
  onBackendConnection: (callback: (connection: BackendConnection) => void) => () => void
  onBackendLog: (callback: (log: BackendLogEvent) => void) => () => void
}
