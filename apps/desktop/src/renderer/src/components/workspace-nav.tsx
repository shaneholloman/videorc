import {
  Broadcast,
  FilmReel,
  GearSix,
  Monitor,
  Record,
  Sparkle,
  SquaresFour,
  VideoCamera,
  type Icon
} from '@phosphor-icons/react'
import { createContext, useContext } from 'react'

// Studio control pages, grouped under "Studio" in the sidebar: one click away, but
// they are FULL pages — studio content renders only on the Studio tab (user decision
// 2026-06-09, overriding the earlier push-rail idea). Sources is the single home for
// every capture device — screen/window, camera, AND microphone — so changing what
// gets captured never requires hunting across pages (UI rewrite plan, 2026-06-10).
export type StudioPanel = 'sources' | 'layouts' | 'live' | 'recording'

// Full pages: they replace the workspace content area.
export type WorkspaceTab = 'studio' | StudioPanel | 'library' | 'ai' | 'diagnostics' | 'settings'

export type WorkspaceTabGroup = 'primary' | 'system'

export type WorkspaceTabMeta = {
  id: WorkspaceTab
  label: string
  icon: Icon
  group: WorkspaceTabGroup
}

export type StudioPanelMeta = {
  id: StudioPanel
  label: string
  icon: Icon
  // The pre-rail tab id; kept as the `data-videorc-tab-trigger` value so smokes and
  // automation keep working across the C1 shell change.
  legacyTabId: string
}

export const WORKSPACE_TABS: WorkspaceTabMeta[] = [
  { id: 'studio', label: 'Studio', icon: VideoCamera, group: 'primary' },
  { id: 'library', label: 'Library', icon: FilmReel, group: 'primary' },
  { id: 'ai', label: 'AI', icon: Sparkle, group: 'primary' },
  { id: 'settings', label: 'Settings', icon: GearSix, group: 'system' }
]

// Sidebar order mirrors the live workflow: pick sources, compose, go live, output.
// There is no Audio page — the microphone and mixer live on Sources with every
// other capture device.
export const STUDIO_PANELS: StudioPanelMeta[] = [
  { id: 'sources', label: 'Sources', icon: Monitor, legacyTabId: 'sources' },
  { id: 'layouts', label: 'Layouts', icon: SquaresFour, legacyTabId: 'layout' },
  { id: 'live', label: 'Live', icon: Broadcast, legacyTabId: 'streaming' },
  { id: 'recording', label: 'Recording', icon: Record, legacyTabId: 'recording' }
]

export const WORKSPACE_GROUPS: { id: WorkspaceTabGroup; label?: string }[] = [
  { id: 'primary' },
  { id: 'system' }
]

export function isStudioPanel(value: unknown): value is StudioPanel {
  return STUDIO_PANELS.some((panel) => panel.id === value)
}

type WorkspaceNavValue = {
  active: WorkspaceTab
  setActive: (tab: WorkspaceTab) => void
  activeStudioPanel: StudioPanel | null
  openStudioPanel: (panel: StudioPanel) => void
  closeStudioPanel: () => void
}

export const WorkspaceNavContext = createContext<WorkspaceNavValue | null>(null)

export function useWorkspaceNav(): WorkspaceNavValue {
  const value = useContext(WorkspaceNavContext)
  if (!value) {
    throw new Error('useWorkspaceNav must be used within a WorkspaceNavContext provider')
  }
  return value
}
