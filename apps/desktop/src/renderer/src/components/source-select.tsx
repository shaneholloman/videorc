import { useId, type ReactElement, type ReactNode } from 'react'

import { Field, FieldDescription, FieldLabel } from '@/components/ui/field'
import {
  Select,
  SelectContent,
  SelectGroup,
  SelectItem,
  SelectTrigger,
  SelectValue
} from '@/components/ui/select'
import type { Device } from '@/lib/backend'

const NONE_VALUE = '__none__'

export function SourceSelect({
  label,
  devices,
  value,
  onChange,
  allowNone = false,
  placeholder = 'Select a device',
  description,
  disabled = false
}: {
  label: string
  devices: Device[]
  value?: string
  onChange: (value: string | undefined) => void
  allowNone?: boolean
  placeholder?: string
  description?: ReactNode
  disabled?: boolean
}): ReactElement {
  const id = useId()

  return (
    <Field>
      <FieldLabel htmlFor={id}>{label}</FieldLabel>
      <Select
        disabled={disabled}
        value={value ?? (allowNone ? NONE_VALUE : '')}
        onValueChange={(next) => onChange(next === NONE_VALUE || next === '' ? undefined : next)}
      >
        <SelectTrigger id={id} className="w-full">
          <SelectValue placeholder={placeholder} />
        </SelectTrigger>
        <SelectContent align="start" position="popper">
          <SelectGroup>
            {allowNone ? <SelectItem value={NONE_VALUE}>None</SelectItem> : null}
            {devices.map((device) => (
              <SelectItem
                disabled={device.status !== 'available'}
                key={device.id}
                value={device.id}
              >
                {device.name}
                {device.status !== 'available' ? ` (${device.status})` : ''}
              </SelectItem>
            ))}
          </SelectGroup>
        </SelectContent>
      </Select>
      {description ? <FieldDescription>{description}</FieldDescription> : null}
    </Field>
  )
}
