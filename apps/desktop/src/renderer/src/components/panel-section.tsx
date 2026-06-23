import type { Icon } from '@phosphor-icons/react'
import type { ReactElement, ReactNode } from 'react'

import { cn } from '@/lib/utils'

/**
 * Glass section (videorc-design): content sits directly on the panel inside a
 * hairline boundary — no opaque card-on-card surfaces. The single section
 * treatment for every tab.
 */
export function PanelSection({
  title,
  description,
  icon: LeadingIcon,
  action,
  children,
  className,
  contentClassName
}: {
  title: string
  description?: ReactNode
  icon?: Icon
  action?: ReactNode
  children: ReactNode
  className?: string
  contentClassName?: string
}): ReactElement {
  return (
    <section
      className={cn('flex flex-col gap-4 rounded-panel border border-border p-4', className)}
      data-slot="panel-section"
    >
      <header className="flex items-start justify-between gap-3">
        <div className="flex min-w-0 flex-col gap-1">
          <h3 className="flex items-center gap-2 text-sm leading-none font-medium text-foreground">
            {LeadingIcon ? (
              <LeadingIcon className="size-4 text-muted-foreground" weight="duotone" />
            ) : null}
            {title}
          </h3>
          {description ? (
            <div className="text-[13px] text-muted-foreground">{description}</div>
          ) : null}
        </div>
        {action ? <div className="shrink-0">{action}</div> : null}
      </header>
      <div className={cn('flex flex-col gap-4', contentClassName)}>{children}</div>
    </section>
  )
}
