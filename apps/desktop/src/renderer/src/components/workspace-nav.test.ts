import { describe, expect, it } from 'vitest'

import {
  STUDIO_PANELS,
  WORKSPACE_SHORTCUTS,
  WORKSPACE_TABS,
  isStudioPanel,
  shortcutDigitFor,
  workspaceTabLabel,
  type WorkspaceTab
} from './workspace-nav'

// The Assets Tab plan (2026-06-15) made Assets a first-class Setup page at ⌘4 and
// pushed Settings off the digit slots onto ⌘,. These invariants guard the IA so a
// later edit can't silently drop a page, duplicate a shortcut, or rename a legacy
// trigger id that smokes/deep-links depend on.
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

  it('maps ⌘1–⌘9 to the nine workflow pages in sidebar order', () => {
    const digitShortcuts = WORKSPACE_SHORTCUTS.filter((entry) => entry.digit !== ',')
    expect(digitShortcuts.map((entry) => [entry.digit, entry.tab])).toEqual([
      ['1', 'studio'],
      ['2', 'sources'],
      ['3', 'layouts'],
      ['4', 'assets'],
      ['5', 'live'],
      ['6', 'recording'],
      ['7', 'library'],
      ['8', 'ai'],
      ['9', 'diagnostics']
    ])
  })

  it('gives Settings ⌘, instead of a digit, and never duplicates a key', () => {
    expect(shortcutDigitFor('settings')).toBe(',')
    expect(WORKSPACE_SHORTCUTS.filter((entry) => entry.tab === 'settings')).toHaveLength(1)

    const keys = WORKSPACE_SHORTCUTS.map((entry) => entry.digit)
    expect(new Set(keys).size).toBe(keys.length)
  })

  it('assigns exactly one shortcut to every reachable page', () => {
    const reachable: WorkspaceTab[] = [
      ...WORKSPACE_TABS.map((tab) => tab.id),
      ...STUDIO_PANELS.map((panel) => panel.id)
    ]
    for (const tab of reachable) {
      expect(shortcutDigitFor(tab), `${tab} needs a shortcut`).toBeDefined()
    }
    expect(WORKSPACE_SHORTCUTS).toHaveLength(reachable.length)
  })

  it('classifies Assets as a Studio panel and labels it', () => {
    expect(isStudioPanel('assets')).toBe(true)
    expect(isStudioPanel('studio')).toBe(false)
    expect(workspaceTabLabel('assets')).toBe('Assets')
  })
})
