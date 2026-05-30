import {
  createContext,
  useCallback,
  useContext,
  useEffect,
  useMemo,
  useRef,
  useState,
  type Dispatch,
  type ReactElement,
  type ReactNode,
  type SetStateAction
} from 'react'
import { toast } from 'sonner'

import { BackendClient } from '@/backendClient'
import {
  defaultSettings,
  loadCaptureConfig,
  loadJson,
  rtmpDefaults,
  STORAGE_KEYS,
  videoPresets,
  type CaptureConfig,
  type SettingsState,
  type SetupStep,
  type WsStatus
} from '@/lib/capture'
import type {
  AiWorkflowResult,
  AudioMeterResult,
  BackendConnection,
  BackendHealth,
  BackendLogEvent,
  Device,
  DeviceList,
  ExportPublishPackResult,
  HealthEvent,
  LayoutSettings,
  PreviewSnapshot,
  RecordingStatus,
  RtmpPreset,
  SessionSummary,
  StartSessionParams,
  StreamHealth,
  VideoPreset,
  VideoSettings
} from '@/lib/backend'
import {
  findDevice,
  durationLabel,
  isActiveRecordingState,
  mergeStreamHealth,
  setupChecklist
} from '@/lib/format'

export type StudioContextValue = {
  // connection + backend state
  connection: BackendConnection | null
  wsStatus: WsStatus
  health: BackendHealth | null
  deviceList: DeviceList
  recording: RecordingStatus
  logs: BackendLogEvent[]
  healthEvents: HealthEvent[]
  streamHealth: StreamHealth | null
  sessions: SessionSummary[]
  // preview + audio
  previewUrl: string | null
  previewLoading: boolean
  audioMeter: AudioMeterResult | null
  audioMeterLoading: boolean
  // ai + jobs
  aiConsent: boolean
  setAiConsent: Dispatch<SetStateAction<boolean>>
  aiRunningSessionId: string | null
  exportRunningSessionId: string | null
  startRequestPending: boolean
  stopRequestPending: boolean
  // settings + capture config
  settings: SettingsState
  setSettings: Dispatch<SetStateAction<SettingsState>>
  captureConfig: CaptureConfig
  setCaptureConfig: Dispatch<SetStateAction<CaptureConfig>>
  patchLayout: (patch: Partial<LayoutSettings>) => void
  patchVideo: (patch: Partial<VideoSettings>) => void
  applyVideoPreset: (preset: VideoPreset) => void
  applyRtmpPreset: (preset: RtmpPreset) => void
  // notices
  lastError: string | null
  // actions
  refreshBackend: () => Promise<void>
  refreshPreview: () => Promise<void>
  sampleAudioMeter: () => Promise<void>
  startSession: () => Promise<void>
  stopSession: () => Promise<void>
  remuxSession: (sessionId: string) => Promise<void>
  runAiWorkflow: (sessionId: string) => Promise<void>
  exportPublishPack: (sessionId: string) => Promise<void>
  // derived
  outputEnabled: boolean
  streamReady: boolean
  isSessionActive: boolean
  startBlockedReason: string | null
  canStart: boolean
  canStop: boolean
  visibleStartBlockedReason: string | null
  selectedCaptureDevice?: Device
  selectedCamera?: Device
  selectedMicrophone?: Device
  setupSteps: SetupStep[]
  elapsed: string
  meterLevel: number
  canSampleAudio: boolean
}

const StudioContext = createContext<StudioContextValue | null>(null)

export function useStudio(): StudioContextValue {
  const value = useContext(StudioContext)
  if (!value) {
    throw new Error('useStudio must be used within a StudioProvider')
  }
  return value
}

export function StudioProvider({ children }: { children: ReactNode }): ReactElement {
  const [connection, setConnection] = useState<BackendConnection | null>(null)
  const [client, setClient] = useState<BackendClient | null>(null)
  const [wsStatus, setWsStatus] = useState<WsStatus>('waiting')
  const [health, setHealth] = useState<BackendHealth | null>(null)
  const [deviceList, setDeviceList] = useState<DeviceList>({ devices: [], warnings: [] })
  const [recording, setRecording] = useState<RecordingStatus>({ state: 'idle', message: 'Ready.' })
  const [logs, setLogs] = useState<BackendLogEvent[]>([])
  const [healthEvents, setHealthEvents] = useState<HealthEvent[]>([])
  const [streamHealth, setStreamHealth] = useState<StreamHealth | null>(null)
  const [sessions, setSessions] = useState<SessionSummary[]>([])
  const [previewUrl, setPreviewUrl] = useState<string | null>(null)
  const [previewLoading, setPreviewLoading] = useState(false)
  const [audioMeter, setAudioMeter] = useState<AudioMeterResult | null>(null)
  const [audioMeterLoading, setAudioMeterLoading] = useState(false)
  const [now, setNow] = useState(() => Date.now())
  const [aiConsent, setAiConsent] = useState(false)
  const [aiRunningSessionId, setAiRunningSessionId] = useState<string | null>(null)
  const [exportRunningSessionId, setExportRunningSessionId] = useState<string | null>(null)
  const [startRequestPending, setStartRequestPending] = useState(false)
  const [stopRequestPending, setStopRequestPending] = useState(false)
  const [settings, setSettings] = useState<SettingsState>(() => loadJson(STORAGE_KEYS.settings, defaultSettings))
  const [captureConfig, setCaptureConfig] = useState<CaptureConfig>(loadCaptureConfig)
  const [lastError, setLastError] = useState<string | null>(null)
  const previewRequestPending = useRef(false)

  const sessionParams = useMemo<StartSessionParams>(
    () => ({
      sources: captureConfig.sources,
      layout: captureConfig.layout,
      output: {
        recordEnabled: captureConfig.recordEnabled,
        streamEnabled: captureConfig.streamEnabled,
        outputDirectory: settings.outputDirectory.trim() || undefined,
        ffmpegPath: settings.ffmpegPath.trim() || undefined,
        video: captureConfig.video,
        rtmp: {
          preset: captureConfig.rtmpPreset,
          serverUrl: captureConfig.rtmpServerUrl.trim(),
          streamKey: captureConfig.streamKey.trim()
        }
      }
    }),
    [captureConfig, settings]
  )

  const reportError = useCallback((error: unknown) => {
    const message = error instanceof Error ? error.message : String(error)
    setLastError(message)
    toast.error(message)
  }, [])

  const appendLog = useCallback((log: BackendLogEvent) => {
    setLogs((current) => [...current.slice(-79), log])
  }, [])

  const refreshSessions = useCallback(async (activeClient: BackendClient | null) => {
    if (!activeClient) {
      return
    }

    const nextSessions = await activeClient.request<SessionSummary[]>('sessions.list')
    setSessions(nextSessions)
  }, [])

  useEffect(() => {
    localStorage.setItem(STORAGE_KEYS.settings, JSON.stringify(settings))
  }, [settings])

  useEffect(() => {
    localStorage.setItem(STORAGE_KEYS.captureConfig, JSON.stringify(captureConfig))
  }, [captureConfig])

  useEffect(() => {
    if (!['recording', 'streaming', 'starting', 'stopping'].includes(recording.state)) {
      return
    }

    const timer = window.setInterval(() => setNow(Date.now()), 1000)
    return () => window.clearInterval(timer)
  }, [recording.state])

  useEffect(() => {
    setAudioMeter(null)
  }, [captureConfig.sources.microphoneId])

  useEffect(() => {
    let disposed = false

    if (typeof window === 'undefined' || !window.videogre) {
      // The preload bridge is unavailable (e.g. rendered outside Electron).
      return
    }

    window.videogre.getBackendLogs().then((backendLogs) => {
      if (!disposed) {
        setLogs(backendLogs.slice(-80))
      }
    })
    window.videogre.getBackendConnection().then((nextConnection) => {
      if (!disposed && nextConnection) {
        setConnection(nextConnection)
      }
    })

    const offConnection = window.videogre.onBackendConnection(setConnection)
    const offLog = window.videogre.onBackendLog(appendLog)

    return () => {
      disposed = true
      offConnection()
      offLog()
    }
  }, [appendLog])

  useEffect(() => {
    setCaptureConfig((current) => {
      const nextSources = { ...current.sources }
      const captureDevices = deviceList.devices.filter(
        (device) => ['screen', 'window'].includes(device.kind) && device.status === 'available'
      )
      const cameras = deviceList.devices.filter((device) => device.kind === 'camera' && device.status === 'available')
      const microphones = deviceList.devices.filter(
        (device) => device.kind === 'microphone' && device.status === 'available'
      )

      nextSources.screenId ||= captureDevices[0]?.id
      nextSources.cameraId ||= cameras[0]?.id
      nextSources.microphoneId ||= microphones[0]?.id

      if (JSON.stringify(nextSources) === JSON.stringify(current.sources)) {
        return current
      }

      return { ...current, sources: nextSources }
    })
  }, [deviceList])

  useEffect(() => {
    if (!connection) {
      return
    }

    const nextClient = new BackendClient(connection)
    setClient(nextClient)
    setWsStatus('connecting')
    setLastError(null)

    const unsubscribers = [
      nextClient.on('backend.ready', () => setWsStatus('connected')),
      nextClient.on('devices.changed', (payload) => setDeviceList(payload as DeviceList)),
      nextClient.on('recording.status', (payload) => {
        const status = payload as RecordingStatus
        setRecording(status)
        if (['idle', 'failed'].includes(status.state)) {
          void refreshSessions(nextClient)
        }
      }),
      nextClient.on('health.event', (payload) => {
        setHealthEvents((current) => [payload as HealthEvent, ...current].slice(0, 40))
      }),
      nextClient.on('stream.health', (payload) => {
        setStreamHealth((current) => mergeStreamHealth(current, payload as StreamHealth))
      }),
      nextClient.on('ai.artifacts.changed', () => {
        void refreshSessions(nextClient)
      }),
      nextClient.on('log', (payload) => appendLog(payload as BackendLogEvent)),
      nextClient.on('error', (payload) => {
        const error = payload as { message?: string }
        const message = error.message ?? 'Backend error.'
        setLastError(message)
        toast.error(message)
      }),
      nextClient.on('connection.closed', () => setWsStatus('closed'))
    ]

    nextClient
      .connect()
      .then(async () => {
        setWsStatus('connected')
        const nextHealth = await nextClient.request<BackendHealth>('health.ping', {
          ffmpegPath: settings.ffmpegPath.trim() || undefined
        })
        setHealth(nextHealth)
        const nextDevices = await nextClient.request<DeviceList>('devices.list', {
          ffmpegPath: settings.ffmpegPath.trim() || undefined
        })
        setDeviceList(nextDevices)
        const nextRecording = await nextClient.request<RecordingStatus>('recording.status')
        setRecording(nextRecording)
        await refreshSessions(nextClient)
      })
      .catch((error: unknown) => {
        setWsStatus('failed')
        reportError(error)
      })

    return () => {
      for (const unsubscribe of unsubscribers) {
        unsubscribe()
      }
      nextClient.close()
      setClient(null)
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [appendLog, connection, refreshSessions, reportError, settings.ffmpegPath])

  const refreshBackend = useCallback(async () => {
    if (!client) {
      return
    }

    try {
      setLastError(null)
      const [nextHealth, nextDevices, nextSessions] = await Promise.all([
        client.request<BackendHealth>('health.ping', { ffmpegPath: settings.ffmpegPath.trim() || undefined }),
        client.request<DeviceList>('devices.list', { ffmpegPath: settings.ffmpegPath.trim() || undefined }),
        client.request<SessionSummary[]>('sessions.list')
      ])
      setHealth(nextHealth)
      setDeviceList(nextDevices)
      setSessions(nextSessions)
    } catch (error) {
      reportError(error)
    }
  }, [client, reportError, settings.ffmpegPath])

  const refreshPreview = useCallback(async () => {
    if (!client || wsStatus !== 'connected' || previewRequestPending.current) {
      return
    }

    try {
      previewRequestPending.current = true
      setPreviewLoading(true)
      const snapshot = await client.request<PreviewSnapshot>('preview.snapshot', {
        sources: captureConfig.sources,
        layout: captureConfig.layout,
        ffmpegPath: settings.ffmpegPath.trim() || undefined
      })
      setPreviewUrl(`${snapshot.url}&cache=${Date.now()}`)
    } catch (error) {
      reportError(error)
    } finally {
      previewRequestPending.current = false
      setPreviewLoading(false)
    }
  }, [captureConfig.layout, captureConfig.sources, client, reportError, settings.ffmpegPath, wsStatus])

  const sampleAudioMeter = useCallback(async () => {
    if (!client) {
      return
    }

    try {
      setLastError(null)
      setAudioMeterLoading(true)
      const result = await client.request<AudioMeterResult>('audio.meter.sample', {
        microphoneId: captureConfig.sources.microphoneId,
        ffmpegPath: settings.ffmpegPath.trim() || undefined
      })
      setAudioMeter(result)
    } catch (error) {
      reportError(error)
    } finally {
      setAudioMeterLoading(false)
    }
  }, [captureConfig.sources.microphoneId, client, reportError, settings.ffmpegPath])

  useEffect(() => {
    if (!client || wsStatus !== 'connected') {
      return
    }

    const timer = window.setTimeout(() => {
      void refreshPreview()
    }, 800)

    return () => window.clearTimeout(timer)
  }, [client, refreshPreview, wsStatus])

  const outputEnabled = captureConfig.recordEnabled || captureConfig.streamEnabled
  const streamReady =
    !captureConfig.streamEnabled || Boolean(captureConfig.rtmpServerUrl.trim() && captureConfig.streamKey.trim())
  const isSessionActive = isActiveRecordingState(recording.state) || startRequestPending || stopRequestPending
  const startBlockedReason = (() => {
    if (wsStatus !== 'connected') {
      return `Backend socket is ${wsStatus}.`
    }
    if (isSessionActive) {
      return 'A capture session is already active.'
    }
    if (!outputEnabled) {
      return 'Enable Record MKV, Stream RTMP, or both before starting.'
    }
    if (captureConfig.streamEnabled && !captureConfig.rtmpServerUrl.trim()) {
      return 'Enter an RTMP server before streaming.'
    }
    if (captureConfig.streamEnabled && !captureConfig.streamKey.trim()) {
      return 'Enter a stream key before streaming, or turn Stream RTMP off.'
    }
    if (!health) {
      return 'Checking FFmpeg before starting.'
    }
    if (!health.ffmpeg.available) {
      return health.ffmpeg.message ?? 'FFmpeg is not available.'
    }

    return null
  })()

  const startSession = useCallback(async () => {
    if (!client || startBlockedReason) {
      if (startBlockedReason && !isSessionActive) {
        reportError(new Error(startBlockedReason))
      }
      return
    }

    try {
      setLastError(null)
      setStreamHealth(null)
      setStartRequestPending(true)
      setRecording((current) =>
        isActiveRecordingState(current.state)
          ? current
          : { state: 'starting', message: 'Starting capture session.' }
      )
      const status = await client.request<RecordingStatus>('session.start', sessionParams)
      setRecording(status)
      await refreshSessions(client)
    } catch (error) {
      reportError(error)
      setRecording((current) =>
        current.state === 'starting' && !current.sessionId
          ? { state: 'idle', message: 'Ready to start a capture session.' }
          : current
      )
    } finally {
      setStartRequestPending(false)
    }
  }, [client, isSessionActive, refreshSessions, reportError, sessionParams, startBlockedReason])

  const stopSession = useCallback(async () => {
    if (!client || stopRequestPending) {
      return
    }

    try {
      setLastError(null)
      setStopRequestPending(true)
      const status = await client.request<RecordingStatus>('session.stop')
      setRecording(status)
    } catch (error) {
      reportError(error)
    } finally {
      setStopRequestPending(false)
    }
  }, [client, reportError, stopRequestPending])

  const remuxSession = useCallback(
    async (sessionId: string) => {
      if (!client) {
        return
      }

      try {
        setLastError(null)
        await client.request('session.remux_mp4', {
          sessionId,
          ffmpegPath: settings.ffmpegPath.trim() || undefined
        })
        await refreshSessions(client)
        toast.success('Remuxed recording to MP4.')
      } catch (error) {
        reportError(error)
      }
    },
    [client, refreshSessions, reportError, settings.ffmpegPath]
  )

  const runAiWorkflow = useCallback(
    async (sessionId: string) => {
      if (!client) {
        return
      }

      try {
        setLastError(null)
        setAiRunningSessionId(sessionId)
        await client.request<AiWorkflowResult>('ai.run_post_recording', {
          sessionId,
          consentToUploadAudio: aiConsent,
          ffmpegPath: settings.ffmpegPath.trim() || undefined
        })
        await refreshSessions(client)
        toast.success(aiConsent ? 'AI workflow finished.' : 'Extracted local audio. Enable consent for cloud AI.')
      } catch (error) {
        reportError(error)
      } finally {
        setAiRunningSessionId(null)
      }
    },
    [aiConsent, client, refreshSessions, reportError, settings.ffmpegPath]
  )

  const exportPublishPack = useCallback(
    async (sessionId: string) => {
      if (!client) {
        return
      }

      try {
        setLastError(null)
        setExportRunningSessionId(sessionId)
        const result = await client.request<ExportPublishPackResult>('ai.publish_pack.export', {
          sessionId
        })
        toast.success(`Publish pack exported to ${result.markdownPath}`)
      } catch (error) {
        reportError(error)
      } finally {
        setExportRunningSessionId(null)
      }
    },
    [client, reportError]
  )

  const patchLayout = useCallback((patch: Partial<LayoutSettings>) => {
    setCaptureConfig((current) => ({ ...current, layout: { ...current.layout, ...patch } }))
  }, [])

  const patchVideo = useCallback((patch: Partial<VideoSettings>) => {
    setCaptureConfig((current) => ({
      ...current,
      video: { ...current.video, ...patch, preset: 'custom' }
    }))
  }, [])

  const applyVideoPreset = useCallback((preset: VideoPreset) => {
    setCaptureConfig((current) => ({ ...current, video: videoPresets[preset] }))
  }, [])

  const applyRtmpPreset = useCallback((preset: RtmpPreset) => {
    setCaptureConfig((current) => ({
      ...current,
      rtmpPreset: preset,
      rtmpServerUrl: rtmpDefaults[preset] || current.rtmpServerUrl
    }))
  }, [])

  const canStart = !startBlockedReason
  const canStop =
    wsStatus === 'connected' &&
    ['recording', 'streaming', 'starting', 'stopping'].includes(recording.state) &&
    !stopRequestPending
  const visibleStartBlockedReason =
    startBlockedReason &&
    !lastError &&
    !isActiveRecordingState(recording.state) &&
    !startRequestPending &&
    !stopRequestPending
      ? startBlockedReason
      : null
  const selectedCaptureDevice = findDevice(
    deviceList.devices,
    captureConfig.sources.screenId ?? captureConfig.sources.windowId
  )
  const selectedCamera = findDevice(deviceList.devices, captureConfig.sources.cameraId)
  const selectedMicrophone = findDevice(deviceList.devices, captureConfig.sources.microphoneId)
  const setupSteps = setupChecklist({
    audioMeter,
    captureConfig,
    health,
    selectedCaptureDevice,
    selectedCamera,
    selectedMicrophone,
    streamReady,
    wsStatus
  })
  const elapsed = recording.startedAt ? durationLabel(recording.startedAt, now) : '00:00'
  const meterLevel = Math.round((audioMeter?.level ?? 0) * 100)
  const canSampleAudio = Boolean(wsStatus === 'connected' && selectedMicrophone && !isSessionActive)

  useEffect(() => {
    const handleKeyDown = (event: KeyboardEvent) => {
      if (event.repeat || isEditableTargetSafe(event.target)) {
        return
      }

      if (event.code === 'Space') {
        event.preventDefault()
        if (canStop) {
          void stopSession()
          return
        }
        if (canStart) {
          void startSession()
        }
        return
      }

      if (event.key.toLowerCase() === 'p') {
        event.preventDefault()
        void refreshPreview()
      }
    }

    window.addEventListener('keydown', handleKeyDown)
    return () => window.removeEventListener('keydown', handleKeyDown)
  }, [canStart, canStop, refreshPreview, startSession, stopSession])

  const value: StudioContextValue = {
    connection,
    wsStatus,
    health,
    deviceList,
    recording,
    logs,
    healthEvents,
    streamHealth,
    sessions,
    previewUrl,
    previewLoading,
    audioMeter,
    audioMeterLoading,
    aiConsent,
    setAiConsent,
    aiRunningSessionId,
    exportRunningSessionId,
    startRequestPending,
    stopRequestPending,
    settings,
    setSettings,
    captureConfig,
    setCaptureConfig,
    patchLayout,
    patchVideo,
    applyVideoPreset,
    applyRtmpPreset,
    lastError,
    refreshBackend,
    refreshPreview,
    sampleAudioMeter,
    startSession,
    stopSession,
    remuxSession,
    runAiWorkflow,
    exportPublishPack,
    outputEnabled,
    streamReady,
    isSessionActive,
    startBlockedReason,
    canStart,
    canStop,
    visibleStartBlockedReason,
    selectedCaptureDevice,
    selectedCamera,
    selectedMicrophone,
    setupSteps,
    elapsed,
    meterLevel,
    canSampleAudio
  }

  return <StudioContext.Provider value={value}>{children}</StudioContext.Provider>
}

function isEditableTargetSafe(target: EventTarget | null): boolean {
  return target instanceof HTMLElement
    ? Boolean(target.closest('input, textarea, select, button, [contenteditable="true"]'))
    : false
}
