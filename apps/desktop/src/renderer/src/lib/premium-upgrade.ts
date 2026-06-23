import type { GoLivePreflight } from './backend'

export const VIDEORC_PREMIUM_URL = 'https://videorc.com/premium'

const PREMIUM_MESSAGE_RE = /\b(?:Videorc\s+)?Premium\b/i

export function isPremiumUpgradeMessage(message: string | null | undefined): boolean {
  return PREMIUM_MESSAGE_RE.test(message ?? '')
}

export function premiumRequiredIssueMessage(
  preflight: Pick<GoLivePreflight, 'issues'>
): string | null {
  return (
    preflight.issues.find(
      (issue) => issue.severity === 'error' && isPremiumUpgradeMessage(issue.message)
    )?.message ?? null
  )
}
