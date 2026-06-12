import {
  ArrowRight,
  Broadcast,
  CheckCircle,
  FileVideo,
  GearSix,
  Monitor,
  ShieldCheck,
  Sparkle,
  Warning,
  Waveform,
  type Icon
} from '@phosphor-icons/react'
import { useEffect, useMemo, useState, type ReactElement } from 'react'

import logoUrl from '@/assets/videorc-logo.png'
import { StatusBadge, type StatusTone } from '@/components/status-badge'
import { Alert, AlertDescription, AlertTitle } from '@/components/ui/alert'
import { Button } from '@/components/ui/button'
import { Checkbox } from '@/components/ui/checkbox'
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle
} from '@/components/ui/dialog'
import {
  Field,
  FieldContent,
  FieldDescription,
  FieldGroup,
  FieldLabel
} from '@/components/ui/field'
import { Separator } from '@/components/ui/separator'
import { ToggleGroup, ToggleGroupItem } from '@/components/ui/toggle-group'
import type { StudioPanel, WorkspaceTab } from '@/components/workspace-nav'
import { useStudio } from '@/hooks/use-studio'
import { rtmpDefaults, videoPresets, type SetupStep } from '@/lib/capture'
import { cn } from '@/lib/utils'

type Workflow = 'record' | 'stream'
type OnboardingStep = 'workflow' | 'setup' | 'privacy' | 'finish'

const STEPS: OnboardingStep[] = ['workflow', 'setup', 'privacy', 'finish']

export function OnboardingDialog({
  open,
  onComplete
}: {
  open: boolean
  onComplete: (target?: WorkspaceTab | StudioPanel) => void
}): ReactElement {
  const {
    health,
    settings,
    captureConfig,
    setCaptureConfig,
    setupSteps,
    streamReady,
    selectedCaptureDevice,
    selectedMicrophone,
    sampleAudioMeter,
    audioMeterLoading,
    canSampleAudio
  } = useStudio()
  const [stepIndex, setStepIndex] = useState(0)
  const [workflow, setWorkflow] = useState<Workflow>(() =>
    captureConfig.streamEnabled ? 'stream' : 'record'
  )
  const [privacyAcknowledged, setPrivacyAcknowledged] = useState(false)

  useEffect(() => {
    if (!open) {
      return
    }

    setStepIndex(0)
    setWorkflow(captureConfig.streamEnabled ? 'stream' : 'record')
    setPrivacyAcknowledged(false)
  }, [captureConfig.streamEnabled, open])

  const step = STEPS[stepIndex]
  const recommendedTab = useMemo<WorkspaceTab | StudioPanel>(() => {
    if (!health?.ffmpeg.available) {
      return 'settings'
    }
    if (!selectedCaptureDevice) {
      return 'sources'
    }
    if (workflow === 'stream' && !streamReady) {
      return 'live'
    }
    return 'studio'
  }, [health?.ffmpeg.available, selectedCaptureDevice, streamReady, workflow])

  const selectWorkflow = (nextWorkflow: Workflow): void => {
    setWorkflow(nextWorkflow)
    setCaptureConfig((current) => {
      const streamPreset =
        current.rtmpPreset === 'custom' && !current.rtmpServerUrl.trim()
          ? 'youtube'
          : current.rtmpPreset

      return {
        ...current,
        recordEnabled: true,
        streamEnabled: nextWorkflow === 'stream',
        video:
          nextWorkflow === 'stream'
            ? videoPresets['stream-1080p60']
            : videoPresets['tutorial-1440p30'],
        rtmpPreset: nextWorkflow === 'stream' ? streamPreset : current.rtmpPreset,
        rtmpServerUrl:
          nextWorkflow === 'stream'
            ? current.rtmpServerUrl || rtmpDefaults[streamPreset] || rtmpDefaults.youtube
            : current.rtmpServerUrl
      }
    })
  }

  const canAdvance = step !== 'privacy' || privacyAcknowledged
  const advance = (): void => {
    if (stepIndex < STEPS.length - 1) {
      setStepIndex((current) => current + 1)
      return
    }

    onComplete(recommendedTab)
  }
  const back = (): void => setStepIndex((current) => Math.max(0, current - 1))

  return (
    <Dialog open={open} onOpenChange={(next) => !next && onComplete()}>
      <DialogContent className="sm:max-w-2xl">
        <DialogHeader>
          <div className="flex items-start justify-between gap-4">
            <div className="flex items-center gap-3">
              <img alt="Videorc" className="size-14 object-contain" src={logoUrl} />
              <div className="flex flex-col gap-1">
                <DialogTitle>Set up Videorc</DialogTitle>
                <DialogDescription>
                  Choose the first-session defaults and confirm the local capture path.
                </DialogDescription>
              </div>
            </div>
            <div
              className="flex items-center gap-1.5 pt-1"
              aria-label={`Step ${stepIndex + 1} of ${STEPS.length}`}
            >
              {STEPS.map((stepId, index) => (
                <span
                  key={stepId}
                  className={cn(
                    'h-1.5 rounded-full transition-all',
                    index === stepIndex
                      ? 'w-6 bg-primary'
                      : index < stepIndex
                        ? 'w-1.5 bg-primary/50'
                        : 'w-1.5 bg-muted-foreground/30'
                  )}
                />
              ))}
            </div>
          </div>
        </DialogHeader>

        {step === 'workflow' ? (
          <WorkflowStep workflow={workflow} onSelectWorkflow={selectWorkflow} />
        ) : null}
        {step === 'setup' ? (
          <SetupStepView
            audioMeterLoading={audioMeterLoading}
            canSampleAudio={canSampleAudio}
            selectedMicrophoneName={selectedMicrophone?.name}
            setupSteps={setupSteps}
            workflow={workflow}
            onOpenTab={onComplete}
            onSampleAudio={sampleAudioMeter}
          />
        ) : null}
        {step === 'privacy' ? (
          <PrivacyStep
            acknowledged={privacyAcknowledged}
            ffmpegDetail={
              health?.ffmpeg.available
                ? (health.ffmpeg.version ?? 'FFmpeg is available.')
                : (health?.ffmpeg.message ?? 'FFmpeg status is still pending.')
            }
            outputDirectory={settings.outputDirectory.trim() || '~/Movies/Videorc/Recordings'}
            onAcknowledgedChange={setPrivacyAcknowledged}
          />
        ) : null}
        {step === 'finish' ? (
          <FinishStep recommendedTab={recommendedTab} workflow={workflow} onComplete={onComplete} />
        ) : null}

        <DialogFooter className="items-center sm:justify-between">
          <Button variant="ghost" onClick={() => onComplete()}>
            Skip
          </Button>
          <div className="flex gap-2">
            {stepIndex > 0 ? (
              <Button variant="outline" onClick={back}>
                Back
              </Button>
            ) : null}
            <Button disabled={!canAdvance} onClick={advance}>
              {step === 'finish' ? 'Open workspace' : 'Continue'}
              <ArrowRight data-icon="inline-end" />
            </Button>
          </div>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  )
}

function WorkflowStep({
  workflow,
  onSelectWorkflow
}: {
  workflow: Workflow
  onSelectWorkflow: (workflow: Workflow) => void
}): ReactElement {
  return (
    <FieldGroup>
      <Field>
        <FieldLabel>First session</FieldLabel>
        <FieldDescription>
          Videorc will apply matching output defaults now. These can be changed later.
        </FieldDescription>
        <ToggleGroup
          className="w-full items-stretch"
          orientation="vertical"
          type="single"
          value={workflow}
          variant="outline"
          onValueChange={(value) => value && onSelectWorkflow(value as Workflow)}
        >
          <ToggleGroupItem
            className="h-auto w-full justify-start px-4 py-3 text-left"
            value="record"
          >
            <WorkflowOption
              icon={FileVideo}
              title="Record tutorial"
              detail="Local MKV recording with the tutorial 1440p30 preset."
            />
          </ToggleGroupItem>
          <ToggleGroupItem
            className="h-auto w-full justify-start px-4 py-3 text-left"
            value="stream"
          >
            <WorkflowOption
              icon={Broadcast}
              title="Record while streaming"
              detail="Local MKV plus RTMP using the stream 1080p60 preset."
            />
          </ToggleGroupItem>
        </ToggleGroup>
      </Field>
    </FieldGroup>
  )
}

function SetupStepView({
  setupSteps,
  workflow,
  selectedMicrophoneName,
  canSampleAudio,
  audioMeterLoading,
  onSampleAudio,
  onOpenTab
}: {
  setupSteps: SetupStep[]
  workflow: Workflow
  selectedMicrophoneName?: string
  canSampleAudio: boolean
  audioMeterLoading: boolean
  onSampleAudio: () => Promise<void>
  onOpenTab: (target?: WorkspaceTab | StudioPanel) => void
}): ReactElement {
  const warningCount = setupSteps.filter((item) => item.tone === 'warn').length

  return (
    <div className="flex flex-col gap-4">
      {warningCount ? (
        <Alert variant="warning">
          <Warning weight="fill" />
          <AlertTitle>Setup needs attention</AlertTitle>
          <AlertDescription>
            Sources, Outputs, and Settings own these checks. You can continue and fix them from the
            workspace.
          </AlertDescription>
        </Alert>
      ) : (
        <Alert variant="success">
          <CheckCircle weight="fill" />
          <AlertTitle>Ready for a first session</AlertTitle>
          <AlertDescription>
            The current device and output defaults are ready for{' '}
            {workflow === 'stream' ? 'recording while streaming' : 'recording'}.
          </AlertDescription>
        </Alert>
      )}

      <div className="grid gap-2 sm:grid-cols-2">
        {setupSteps.map((item) => (
          <SetupStatusRow item={item} key={item.label} />
        ))}
      </div>

      <Separator />

      <div className="flex flex-wrap gap-2">
        <Button size="sm" variant="outline" onClick={() => onOpenTab('sources')}>
          <Monitor data-icon="inline-start" />
          Sources
        </Button>
        <Button size="sm" variant="outline" onClick={() => onOpenTab('live')}>
          <Broadcast data-icon="inline-start" />
          Destinations
        </Button>
        <Button size="sm" variant="outline" onClick={() => onOpenTab('settings')}>
          <GearSix data-icon="inline-start" />
          Settings
        </Button>
        <Button
          disabled={!canSampleAudio || audioMeterLoading}
          size="sm"
          variant="outline"
          onClick={onSampleAudio}
        >
          <Waveform data-icon="inline-start" />
          {audioMeterLoading
            ? 'Checking mic'
            : selectedMicrophoneName
              ? 'Check mic'
              : 'No mic selected'}
        </Button>
      </div>
    </div>
  )
}

function PrivacyStep({
  acknowledged,
  ffmpegDetail,
  outputDirectory,
  onAcknowledgedChange
}: {
  acknowledged: boolean
  ffmpegDetail: string
  outputDirectory: string
  onAcknowledgedChange: (checked: boolean) => void
}): ReactElement {
  return (
    <FieldGroup>
      <Field orientation="horizontal">
        <Checkbox
          aria-label="Acknowledge local and cloud workflow"
          checked={acknowledged}
          onCheckedChange={(checked) => onAcknowledgedChange(checked === true)}
        />
        <FieldContent>
          <FieldLabel>Local by default</FieldLabel>
          <FieldDescription>
            Recordings and session metadata stay on this Mac. Cloud AI only runs from the AI tab
            after explicit consent.
          </FieldDescription>
        </FieldContent>
      </Field>

      <div className="grid gap-3 sm:grid-cols-2">
        <Fact icon={FileVideo} label="Recording folder" value={outputDirectory} />
        <Fact icon={GearSix} label="FFmpeg" value={ffmpegDetail} />
        <Fact icon={ShieldCheck} label="AI uploads" value="Opt-in per session" />
        <Fact icon={Sparkle} label="AI artifacts" value="Stored in the local library" />
      </div>
    </FieldGroup>
  )
}

function FinishStep({
  workflow,
  recommendedTab,
  onComplete
}: {
  workflow: Workflow
  recommendedTab: WorkspaceTab | StudioPanel
  onComplete: (target?: WorkspaceTab | StudioPanel) => void
}): ReactElement {
  return (
    <div className="flex flex-col gap-4">
      <Alert>
        <CheckCircle weight="fill" />
        <AlertTitle>
          {workflow === 'stream' ? 'Streaming workflow selected' : 'Recording workflow selected'}
        </AlertTitle>
        <AlertDescription>
          The workspace is ready to continue from the most relevant tab.
        </AlertDescription>
      </Alert>

      <div className="grid gap-2 sm:grid-cols-2">
        <DestinationButton
          recommended={recommendedTab === 'sources'}
          icon={Monitor}
          label="Sources"
          onClick={() => onComplete('sources')}
        />
        <DestinationButton
          recommended={recommendedTab === 'live'}
          icon={Broadcast}
          label="Streaming"
          onClick={() => onComplete('live')}
        />
        <DestinationButton
          recommended={recommendedTab === 'studio'}
          icon={FileVideo}
          label="Studio"
          onClick={() => onComplete('studio')}
        />
        <DestinationButton icon={Sparkle} label="AI" onClick={() => onComplete('ai')} />
      </div>
      {recommendedTab === 'settings' ? (
        <DestinationButton
          recommended
          icon={GearSix}
          label="Settings"
          onClick={() => onComplete('settings')}
        />
      ) : null}
    </div>
  )
}

function WorkflowOption({
  icon: LeadingIcon,
  title,
  detail
}: {
  icon: Icon
  title: string
  detail: string
}): ReactElement {
  return (
    <span className="flex min-w-0 items-start gap-3">
      <LeadingIcon data-icon="inline-start" weight="duotone" />
      <span className="flex min-w-0 flex-col gap-1">
        <span className="font-medium">{title}</span>
        <span className="text-sm text-muted-foreground">{detail}</span>
      </span>
    </span>
  )
}

function SetupStatusRow({ item }: { item: SetupStep }): ReactElement {
  return (
    <div className="flex items-start justify-between gap-3 rounded-lg border bg-muted/30 px-3 py-2">
      <div className="flex min-w-0 flex-col">
        <span className="text-sm font-medium">{item.label}</span>
        <span className="line-clamp-2 text-xs text-muted-foreground">{item.detail}</span>
      </div>
      <StatusBadge
        tone={toneForSetup(item.tone)}
        value={item.tone === 'good' ? 'ready' : item.tone}
      />
    </div>
  )
}

function Fact({
  icon: LeadingIcon,
  label,
  value
}: {
  icon: Icon
  label: string
  value: string
}): ReactElement {
  return (
    <div className="flex min-w-0 gap-3 rounded-lg border bg-muted/30 px-3 py-2">
      <LeadingIcon className="mt-0.5 size-4 shrink-0 text-muted-foreground" weight="duotone" />
      <div className="flex min-w-0 flex-col">
        <span className="text-xs text-muted-foreground">{label}</span>
        <span className="truncate text-sm font-medium">{value}</span>
      </div>
    </div>
  )
}

function DestinationButton({
  icon: LeadingIcon,
  label,
  recommended,
  onClick
}: {
  icon: Icon
  label: string
  recommended?: boolean
  onClick: () => void
}): ReactElement {
  return (
    <Button
      className={cn('h-auto justify-start px-3 py-3', recommended && 'border-primary')}
      variant={recommended ? 'default' : 'outline'}
      onClick={onClick}
    >
      <LeadingIcon data-icon="inline-start" weight="duotone" />
      {recommended ? `${label} first` : label}
    </Button>
  )
}

function toneForSetup(tone: SetupStep['tone']): StatusTone {
  return tone === 'good' ? 'good' : tone === 'warn' ? 'warn' : 'neutral'
}
