// Run: node --test scripts/lib/screen-motion-stimulus.test.mjs

import assert from 'node:assert/strict'
import { describe, it } from 'node:test'

import { stimulusWindowOptionsFromDisplayBounds } from './screen-motion-stimulus.mjs'

describe('stimulusWindowOptionsFromDisplayBounds', () => {
  it('places the stimulus inside a non-primary display with negative y bounds', () => {
    assert.deepEqual(
      stimulusWindowOptionsFromDisplayBounds({ x: 1512, y: -56, width: 1920, height: 1080 }),
      { x: 1528, y: -40, width: 1888, height: 1048 }
    )
  })

  it('keeps a usable minimum window for small or odd display bounds', () => {
    assert.deepEqual(
      stimulusWindowOptionsFromDisplayBounds({ x: 0, y: 0, width: 400, height: 300 }),
      { x: 16, y: 16, width: 640, height: 480 }
    )
  })
})
