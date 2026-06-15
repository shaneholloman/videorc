import { ImageSquare } from '@phosphor-icons/react'
import type { ReactElement } from 'react'

import { Empty, EmptyDescription, EmptyMedia, EmptyTitle } from '@/components/ui/empty'

// Assets owns reusable visual material — background presets, imports, and the
// background inspector. Slice A1 ships the navigation shell only; the preset grid
// (A2), PowerSlider inspector (A3–A4), and compositor-backed output (A5–A6) land
// in later slices. The empty state says so plainly instead of rendering a blank
// page (AGENTS.md: explicit status over silent fallbacks).
export function AssetsTab(): ReactElement {
  return (
    <div className="flex flex-col gap-5">
      <div className="flex flex-col gap-1">
        <h1 className="text-xl font-semibold tracking-tight">Assets</h1>
        <p className="max-w-2xl text-sm text-muted-foreground">
          Reusable visual material for your scenes. Background presets live here; selecting one
          applies it to the active scene, while Scene keeps any per-scene overrides.
        </p>
      </div>
      <Empty className="rounded-xl border py-16">
        <EmptyMedia variant="icon">
          <ImageSquare weight="duotone" />
        </EmptyMedia>
        <EmptyTitle>Background presets are coming next</EmptyTitle>
        <EmptyDescription>
          Ten background slots, still-image import, and PowerSlider controls arrive in the next
          slices.
        </EmptyDescription>
      </Empty>
    </div>
  )
}
