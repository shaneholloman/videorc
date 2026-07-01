import assert from 'node:assert/strict'
import { describe, it } from 'node:test'

import {
  buildRecordingStudioGateSteps,
  formatRecordingStudioGatePlan
} from './recording-studio-gates.mjs'

describe('buildRecordingStudioGateSteps', () => {
  it('covers studio unit tests, script A/V tests, backend studio modules, and app smoke', () => {
    const steps = buildRecordingStudioGateSteps()
    const labels = steps.map((step) => step.label)

    assert.deepEqual(labels, [
      'desktop recording studio unit tests',
      'script artifact analyzer and A/V sync tests',
      'backend live layout tests',
      'backend scene layout tests',
      'backend recording pipeline tests',
      'backend audio pipeline tests',
      'dev app all-layout recording artifact smoke',
      'imported screen image recording smoke',
      'layout/source preview liveness smoke',
      'backend-owned preview scene commit smoke',
      'preview main pump diagnostics smoke',
      'preview click/focus continuity smoke',
      'detached preview lifecycle probe',
      'detached native preview surface reattach smoke',
      'real ScreenCaptureKit screen recording smoke',
      'Notes window recording invisibility smoke'
    ])
    assert.deepEqual(steps[0].args, [
      '--filter',
      '@videorc/desktop',
      'test',
      'capture.test.ts',
      'background-assets.test.ts',
      'session-params.test.ts',
      'studio-health.test.ts',
      'native-preview-present-policy.test.ts'
    ])
    assert.deepEqual(steps[1].args, ['test:scripts'])
    assert.deepEqual(steps.at(-10).args, ['smoke:dev'])
    assert.deepEqual(steps.at(-9).args, ['smoke:screens'])
    assert.deepEqual(steps.at(-8).args, ['smoke:layout-source-loop'])
    assert.deepEqual(steps.at(-7).args, ['smoke:preview-scene-commit'])
    assert.deepEqual(steps.at(-6).args, ['smoke:preview-pump-diagnostics'])
    assert.deepEqual(steps.at(-5).args, ['smoke:preview-click-focus'])
    assert.deepEqual(steps.at(-4).args, ['probe:preview-lifecycle'])
    assert.deepEqual(steps.at(-3).args, ['smoke:preview-surface'])
    assert.equal(steps.at(-3).env.VIDEORC_PREVIEW_SURFACE_MIN_FPS, '30')
    assert.equal(steps.at(-3).env.VIDEORC_PREVIEW_SURFACE_MAX_INTERVAL_P95_MS, '120')
    assert.equal(steps.at(-3).env.VIDEORC_PREVIEW_SURFACE_MAX_INPUT_TO_PRESENT_P95_MS, '100')
    assert.deepEqual(steps.at(-2).args, ['smoke:screen-recording-real'])
    assert.deepEqual(steps.at(-1).args, ['smoke:notes-window-invisible'])
  })

  it('can include the heavier native preview layout-stress smoke', () => {
    const steps = buildRecordingStudioGateSteps({ includeDeviceSmoke: true })
    const deviceSmoke = steps.at(-1)

    assert.equal(deviceSmoke.label, 'native preview source-complete layout stress recording smoke')
    assert.deepEqual(deviceSmoke.args, ['smoke:recording-native-preview'])
    assert.equal(deviceSmoke.env.VIDEORC_NATIVE_PREVIEW_SOURCE_COMPLETE_SCENE, '1')
    assert.equal(deviceSmoke.env.VIDEORC_NATIVE_PREVIEW_LAYOUT_STRESS_UPDATES, '4')
  })

  it('formats commands for dry-run evidence', () => {
    const report = formatRecordingStudioGatePlan({
      steps: buildRecordingStudioGateSteps({ includeDeviceSmoke: true })
    })

    assert.match(report, /recording-studio-gates: plan/)
    assert.match(report, /capture\.test\.ts/)
    assert.match(report, /test:scripts/)
    assert.match(report, /live_layout::tests::/)
    assert.match(report, /smoke:dev/)
    assert.match(report, /smoke:screens/)
    assert.match(report, /smoke:layout-source-loop/)
    assert.match(report, /smoke:preview-scene-commit/)
    assert.match(report, /smoke:preview-pump-diagnostics/)
    assert.match(report, /smoke:preview-click-focus/)
    assert.match(report, /probe:preview-lifecycle/)
    assert.match(report, /smoke:preview-surface/)
    assert.match(report, /smoke:screen-recording-real/)
    assert.match(report, /smoke:notes-window-invisible/)
    assert.match(report, /VIDEORC_PREVIEW_SURFACE_MAX_INPUT_TO_PRESENT_P95_MS=100/)
    assert.match(report, /VIDEORC_NATIVE_PREVIEW_SOURCE_COMPLETE_SCENE=1/)
    assert.match(report, /VIDEORC_NATIVE_PREVIEW_LAYOUT_STRESS_UPDATES=4/)
    assert.match(report, /pnpm smoke:recording-native-preview/)
  })
})
