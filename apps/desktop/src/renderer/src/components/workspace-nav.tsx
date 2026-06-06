import {
  Broadcast,
  FileVideo,
  FilmReel,
  Gauge,
  GearSix,
  ImageSquare,
  Layout,
  Monitor,
  Sparkle,
  VideoCamera,
  type Icon
} from '@phosphor-icons/react'
import { createContext, useContext } from 'react'

export type WorkspaceTab =
  | 'studio'
  | 'sources'
  | 'layout'
  | 'screens'
  | 'recording'
  | 'streaming'
  | 'library'
  | 'ai'
  | 'diagnostics'
  | 'settings'

// Sidebar groups. `primary` is the target nav (Studio/Library/AI + Settings in `system`).
// `setup` is transitional: those tabs fold into the Studio Inspector across later slices and
// disappear from the sidebar as they do.
export type WorkspaceTabGroup = 'primary' | 'setup' | 'system'

export type WorkspaceTabMeta = {
  id: WorkspaceTab
  label: string
  icon: Icon
  group: WorkspaceTabGroup
}

export const WORKSPACE_TABS: WorkspaceTabMeta[] = [
  { id: 'studio', label: 'Studio', icon: VideoCamera, group: 'primary' },
  { id: 'library', label: 'Library', icon: FilmReel, group: 'primary' },
  { id: 'ai', label: 'AI', icon: Sparkle, group: 'primary' },
  { id: 'sources', label: 'Sources', icon: Monitor, group: 'setup' },
  { id: 'screens', label: 'Screens', icon: ImageSquare, group: 'setup' },
  { id: 'layout', label: 'Layout', icon: Layout, group: 'setup' },
  { id: 'recording', label: 'Recording', icon: FileVideo, group: 'setup' },
  { id: 'streaming', label: 'Streaming', icon: Broadcast, group: 'setup' },
  { id: 'settings', label: 'Settings', icon: GearSix, group: 'system' },
  { id: 'diagnostics', label: 'Diagnostics', icon: Gauge, group: 'system' }
]

export const WORKSPACE_GROUPS: { id: WorkspaceTabGroup; label?: string }[] = [
  { id: 'primary' },
  { id: 'setup', label: 'Setup' },
  { id: 'system' }
]

type WorkspaceNavValue = {
  active: WorkspaceTab
  setActive: (tab: WorkspaceTab) => void
}

export const WorkspaceNavContext = createContext<WorkspaceNavValue | null>(null)

export function useWorkspaceNav(): WorkspaceNavValue {
  const value = useContext(WorkspaceNavContext)
  if (!value) {
    throw new Error('useWorkspaceNav must be used within a WorkspaceNavContext provider')
  }
  return value
}
