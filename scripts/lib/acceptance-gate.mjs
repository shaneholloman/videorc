// Real-source acceptance gate (plan Phase 1 / Phase 8).
//
// A pure evaluator that decides whether a real-source recording run passes the
// OBS-quality bar, combining the honest final-file analyzer verdict with the live
// diagnostics sampled during the run. Kept separate from the live harness so the gate
// logic itself is unit-testable on synthetic inputs (no app, no hardware) — which is the
// point of Phase 1: synthetic-only success must not be able to mark the plan complete,
// and the gate must fail the current bad path.

export const DEFAULT_ACCEPTANCE_GATES = Object.freeze({
  minEncoderSpeed: 0.98, // encoder must stay at/above real-time
  minMicCaptureCoverage: 0.95, // mic must capture ≥95% of expected samples
})

/**
 * @param {object} input
 * @param {{pass:boolean, failures:string[]}} input.analyzerVerdict - from analyzeRecording()
 * @param {{pass:boolean, failures:string[]}} [input.startupVerdict] - from analyzeStartupResolution()
 * @param {object} input.diagnostics - summarized live diagnostics for the run
 * @param {boolean} input.claimsNative - whether the preview reported a native transport
 * @param {boolean} input.expectAudio - whether a mic was selected
 * @param {object} [gates]
 * @returns {{pass:boolean, failures:string[]}}
 */
export function evaluateAcceptance(input, gates = DEFAULT_ACCEPTANCE_GATES) {
  const failures = []
  const d = input.diagnostics ?? {}

  // 1. Final-file gates (freeze / repeated-frame bursts / pacing / audio / A/V skew).
  if (input.analyzerVerdict && !input.analyzerVerdict.pass) {
    for (const failure of input.analyzerVerdict.failures ?? []) {
      failures.push(`final-file: ${failure}`)
    }
  }

  // 1b. Startup gates: the first seconds must already be target-resolution real output.
  if (input.startupVerdict && !input.startupVerdict.pass) {
    for (const failure of input.startupVerdict.failures ?? []) {
      failures.push(`startup: ${failure}`)
    }
  }

  // 2. Recording must not contain duplicate frames re-fed to the encoder on under-run.
  if ((d.encoderBridgeRepeatedFrames ?? 0) > 0) {
    failures.push(
      `recording: ${d.encoderBridgeRepeatedFrames} duplicate frame(s) re-fed to the encoder (compositor under-run)`
    )
  }
  if ((d.encoderBridgeSyntheticFrames ?? 0) > 0) {
    failures.push(
      `recording: ${d.encoderBridgeSyntheticFrames} synthetic filler frame(s) fed (no real source ready)`
    )
  }

  // 3. Encoder must keep real-time.
  if (d.minEncoderSpeed != null && d.minEncoderSpeed < gates.minEncoderSpeed) {
    failures.push(
      `encoder: speed ${d.minEncoderSpeed.toFixed(2)}x below ${gates.minEncoderSpeed}x`
    )
  }

  // 4. Audio: zero mic drops and adequate capture coverage.
  if (input.expectAudio) {
    if ((d.micDroppedFrames ?? 0) > 0) {
      failures.push(`audio: microphone dropped ${d.micDroppedFrames} frame(s)`)
    }
    if (d.minMicCaptureCoverage != null && d.minMicCaptureCoverage < gates.minMicCaptureCoverage) {
      failures.push(
        `audio: mic capture coverage ${(d.minMicCaptureCoverage * 100).toFixed(0)}% below ${(gates.minMicCaptureCoverage * 100).toFixed(0)}%`
      )
    }
  }

  // 5. Transport honesty: a "native" preview must not have fetched image-poll routes.
  const imagePolls = d.imagePollDuringSession?.total
  if (input.claimsNative && imagePolls != null && imagePolls > 0) {
    failures.push(
      `transport: ${imagePolls} image-poll request(s) during a "native" preview session (not native)`
    )
  }

  return { pass: failures.length === 0, failures }
}
