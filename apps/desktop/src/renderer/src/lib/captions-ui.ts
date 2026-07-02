import type { CaptionsUpdate } from '@/lib/backend'

/** Lines kept for the captions strip / detached window. */
export const MAX_CAPTION_LINES = 50

/**
 * Append a caption update: streaming PARTIALS (and the final that settles
 * them) REPLACE the line with the same seq; older seqs are dropped
 * (chunked-retry duplicates); a new caption session resets the buffer.
 * Newest line last; capped to MAX_CAPTION_LINES.
 */
export function appendCaptionLine(
  lines: CaptionsUpdate[],
  update: CaptionsUpdate,
  max = MAX_CAPTION_LINES
): CaptionsUpdate[] {
  if (!update.text.trim()) {
    return lines
  }
  const last = lines.at(-1)
  if (last && last.sessionClientId !== update.sessionClientId) {
    return [update]
  }
  if (last && update.seq === last.seq) {
    // The utterance is still evolving (partial → partial → final).
    return [...lines.slice(0, -1), update]
  }
  if (last && update.seq < last.seq) {
    return lines
  }
  return [...lines, update].slice(-max)
}

/** The strip shows the tail of the transcript, most recent lines only. */
export function captionStripLines(lines: CaptionsUpdate[], count = 3): CaptionsUpdate[] {
  return lines.slice(-count)
}
