import { describe, expect, it } from 'vitest'

import {
  STUDIO_PANELS,
  WORKSPACE_SHORTCUTS,
  WORKSPACE_TABS,
  isStudioPanel,
  isWorkspaceTab,
  shortcutDigitFor,
  workspaceTabLabel,
  type WorkspaceTab
} from './workspace-nav'

// Assets is a first-class Setup page at ⌘4. Settings moved onto ⌘8 (2026-06-24),
// and AI + Health (Diagnostics) intentionally have no digit — both stay reachable
// via ⌘K. These invariants guard the IA so a later edit can't silently drop a page,
// duplicate a shortcut, or rename a legacy trigger id that smokes/deep-links depend on.
describe('workspace navigation', () => {
  it('registers Assets as a Setup panel between Scene and Destinations', () => {
    expect(STUDIO_PANELS.map((panel) => panel.id)).toEqual([
      'sources',
      'layouts',
      'assets',
      'live',
      'recording'
    ])

    const assets = STUDIO_PANELS.find((panel) => panel.id === 'assets')
    expect(assets?.label).toBe('Assets')
    expect(assets?.legacyTabId).toBe('assets')
  })

  it('keeps legacy trigger ids stable for existing Setup panels', () => {
    const legacyById = Object.fromEntries(
      STUDIO_PANELS.map((panel) => [panel.id, panel.legacyTabId])
    )
    expect(legacyById).toMatchObject({
      sources: 'sources',
      layouts: 'layout',
      live: 'streaming',
      recording: 'recording'
    })
  })

  it('maps ⌘1–⌘9 to the workflow pages in sidebar order (AI ⌘8, Settings ⌘9)', () => {
    expect(WORKSPACE_SHORTCUTS.map((entry) => [entry.digit, entry.tab])).toEqual([
      ['1', 'studio'],
      ['2', 'sources'],
      ['3', 'layouts'],
      ['4', 'assets'],
      ['5', 'live'],
      ['6', 'recording'],
      ['7', 'library'],
      ['8', 'ai'],
      ['9', 'settings']
    ])
  })

  it('puts Settings on ⌘9 and never duplicates a key', () => {
    expect(shortcutDigitFor('settings')).toBe('9')
    expect(WORKSPACE_SHORTCUTS.filter((entry) => entry.tab === 'settings')).toHaveLength(1)

    const keys = WORKSPACE_SHORTCUTS.map((entry) => entry.digit)
    expect(new Set(keys).size).toBe(keys.length)
  })

  it('gives every reachable page a digit except Health (⌘K-only)', () => {
    const noDigit: WorkspaceTab[] = ['diagnostics']
    const reachable: WorkspaceTab[] = [
      ...WORKSPACE_TABS.map((tab) => tab.id),
      ...STUDIO_PANELS.map((panel) => panel.id)
    ]
    for (const tab of reachable) {
      if (noDigit.includes(tab)) {
        expect(shortcutDigitFor(tab), `${tab} should have no digit`).toBeUndefined()
      } else {
        expect(shortcutDigitFor(tab), `${tab} needs a shortcut`).toBeDefined()
      }
    }
    expect(WORKSPACE_SHORTCUTS).toHaveLength(reachable.length - noDigit.length)
  })

  it('classifies Assets as a Studio panel and labels it', () => {
    expect(isStudioPanel('assets')).toBe(true)
    expect(isStudioPanel('studio')).toBe(false)
    expect(isWorkspaceTab('assets')).toBe(true)
    expect(isWorkspaceTab('library')).toBe(true)
    expect(isWorkspaceTab('missing')).toBe(false)
    expect(workspaceTabLabel('assets')).toBe('Assets')
  })
})
