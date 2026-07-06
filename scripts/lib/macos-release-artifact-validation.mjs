import { basename, relative } from 'node:path'

export function artifactKindFromPath(path) {
  if (String(path).endsWith('.app')) {
    return 'app'
  }
  if (String(path).endsWith('.dmg')) {
    return 'dmg'
  }
  return null
}

// Hardened-runtime AV capture requires these on every binary that touches the
// camera/microphone; a TCC grant cannot override a missing entitlement. 0.9.1
// shipped without them (camera dead in the packaged app), so the validator now
// fails closed when any capture binary lacks them.
export const REQUIRED_CAPTURE_ENTITLEMENTS = [
  'com.apple.security.device.camera',
  'com.apple.security.device.audio-input'
]

// One shared entitlements plist signs the app and every bundled tool, so all of
// them must carry the device entitlements (paths mirror
// scripts/sign-macos-local-app.mjs and electron-builder extraResources).
export function captureEntitlementCheckTargets(appPath) {
  return [
    { id: 'app', label: 'app', path: appPath },
    {
      id: 'videorc-backend',
      label: 'videorc-backend',
      path: `${appPath}/Contents/Resources/videorc-backend`
    },
    {
      id: 'native-preview-host-helper',
      label: 'native_preview_host_helper',
      path: `${appPath}/Contents/Resources/native_preview_host_helper`
    },
    { id: 'ffmpeg', label: 'ffmpeg', path: `${appPath}/Contents/Resources/ffmpeg/bin/ffmpeg` },
    { id: 'ffprobe', label: 'ffprobe', path: `${appPath}/Contents/Resources/ffmpeg/bin/ffprobe` }
  ]
}

export function buildMacosReleaseArtifactChecks(path) {
  const kind = artifactKindFromPath(path)
  if (!kind) {
    throw new Error(`Unsupported macOS release artifact: ${path}`)
  }

  if (kind === 'app') {
    return [
      {
        id: 'codesign-verify',
        label: 'codesign verify',
        command: 'codesign',
        args: ['--verify', '--deep', '--strict', '--verbose=2', path]
      },
      {
        id: 'codesign-display',
        label: 'codesign display',
        command: 'codesign',
        args: ['-dv', '--verbose=4', path]
      },
      {
        id: 'spctl-assess',
        label: 'Gatekeeper assess',
        command: 'spctl',
        args: ['--assess', '--type', 'execute', '--verbose', path]
      },
      {
        id: 'stapler-validate',
        label: 'stapler validate',
        command: 'xcrun',
        args: ['stapler', 'validate', path]
      },
      ...captureEntitlementCheckTargets(path).map((target) => ({
        id: `capture-entitlements-${target.id}`,
        label: `capture entitlements (${target.label})`,
        command: 'codesign',
        args: ['-d', '--entitlements', ':-', target.path],
        expectOutputIncludes: REQUIRED_CAPTURE_ENTITLEMENTS
      })),
      {
        // The YouTube OAuth client secret left source (public repo, 2026-07-06)
        // and is compiled in via VIDEORC_BUNDLED_YOUTUBE_CLIENT_SECRET at release
        // build time. A release backend without it ships broken YouTube connect,
        // so the gate proves the OUTCOME: the binary must embed a GOCSPX- secret
        // (every Google Desktop-client secret carries that prefix).
        id: 'bundled-youtube-oauth-secret',
        label: 'bundled YouTube OAuth secret (videorc-backend)',
        command: 'grep',
        // -a is load-bearing: BSD grep refuses to match inside a binary file
        // without it (verified 2026-07-06 — the check fails on a GOOD build).
        args: ['-q', '-a', 'GOCSPX-', `${path}/Contents/Resources/videorc-backend`]
      }
    ]
  }

  return [
    {
      id: 'codesign-verify',
      label: 'codesign verify',
      command: 'codesign',
      args: ['--verify', '--verbose=2', path]
    },
    {
      id: 'codesign-display',
      label: 'codesign display',
      command: 'codesign',
      args: ['-dv', '--verbose=4', path]
    },
    {
      id: 'spctl-assess',
      label: 'Gatekeeper assess',
      command: 'spctl',
      args: [
        '--assess',
        '--type',
        'open',
        '--context',
        'context:primary-signature',
        '--verbose',
        path
      ]
    },
    {
      id: 'stapler-validate',
      label: 'stapler validate',
      command: 'xcrun',
      args: ['stapler', 'validate', path]
    }
  ]
}

export function selectLatestReleaseArtifacts(candidates) {
  const latestByKind = new Map()

  for (const candidate of candidates) {
    const kind = candidate.kind ?? artifactKindFromPath(candidate.path)
    if (!kind) {
      continue
    }

    const previous = latestByKind.get(kind)
    if (!previous || Number(candidate.mtimeMs ?? 0) > Number(previous.mtimeMs ?? 0)) {
      latestByKind.set(kind, { ...candidate, kind })
    }
  }

  return ['app', 'dmg'].map((kind) => latestByKind.get(kind)).filter(Boolean)
}

export function formatArtifactPath(path, { repoRoot, homeDir } = {}) {
  const raw = String(path)
  if (repoRoot) {
    const rel = relative(repoRoot, raw)
    if (rel && !rel.startsWith('..')) {
      return rel
    }
  }

  if (homeDir && raw.startsWith(homeDir)) {
    return `<home>/${basename(raw)}`
  }

  return `<external>/${basename(raw)}`
}

export function sanitizeReleaseValidationOutput(text, { repoRoot, homeDir } = {}) {
  let output = String(text ?? '')
  if (repoRoot) {
    output = output.split(repoRoot).join('<repo>')
  }
  if (homeDir) {
    output = output.split(homeDir).join('<home>')
  }
  return output
}

export function formatReleaseArtifactValidationReport({ artifactLabel, results }) {
  const ok = results.every((result) => result.ok)
  const lines = [`macos-release-artifact: ${ok ? 'PASS' : 'FAIL'} ${artifactLabel}`]

  for (const result of results) {
    const mark = result.ok ? 'ok' : 'fail'
    lines.push(`[${mark}] ${result.label}`)
    if (!result.ok && result.output) {
      lines.push(indentExcerpt(result.output))
    }
  }

  return lines.join('\n')
}

function indentExcerpt(text) {
  return String(text)
    .trim()
    .split(/\r?\n/)
    .slice(0, 12)
    .map((line) => `  ${line}`)
    .join('\n')
}
