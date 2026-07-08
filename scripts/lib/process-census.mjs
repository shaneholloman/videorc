import { createHash } from 'node:crypto'
import { execFile } from 'node:child_process'
import { existsSync, readFileSync, writeFileSync } from 'node:fs'
import { join } from 'node:path'
import { promisify } from 'node:util'

const execFileAsync = promisify(execFile)

export function ownedProcessLedgerPaths({
  appDataDir,
  userDataDir,
  workspaceRoot,
  appName = 'Videorc'
}) {
  const workspaceKey = createHash('sha256').update(workspaceRoot).digest('hex').slice(0, 16)
  return [
    join(appDataDir, appName, 'owned-processes', 'global.json'),
    join(userDataDir, 'owned-processes', `${workspaceKey}.json`)
  ]
}

export function readOwnedProcessLedgers(ledgerPaths, { readFile = readFileSync } = {}) {
  const seen = new Set()
  const records = []
  for (const ledgerPath of ledgerPaths) {
    let parsed
    try {
      parsed = JSON.parse(readFile(ledgerPath, 'utf8'))
    } catch {
      continue
    }
    if (!Array.isArray(parsed)) {
      continue
    }
    for (const record of parsed) {
      if (!isOwnedProcessRecord(record) || seen.has(record.pid)) {
        continue
      }
      seen.add(record.pid)
      records.push({ ...record, ledgerPath })
    }
  }
  return records
}

export async function collectProcessCensus({
  ledgerPaths,
  pgid,
  extraPids = [],
  readFile = readFileSync,
  readProcessTable = readSystemProcessTable
}) {
  const records = readOwnedProcessLedgers(ledgerPaths, { readFile })
  const processTable = await readProcessTable()
  const rowsByPid = new Map(processTable.map((row) => [row.pid, row]))
  const interestingPids = new Set([...records.map((record) => record.pid), ...extraPids])
  const processGroupRows =
    typeof pgid === 'number' ? processTable.filter((row) => row.pgid === pgid) : []
  const processRows = processTable
    .filter(
      (row) => interestingPids.has(row.pid) || (typeof pgid === 'number' && row.pgid === pgid)
    )
    .map((row) => ({ ...row, role: classifyProcess(row) }))
    .sort((a, b) => a.pid - b.pid)
  const aliveRecords = records.filter((record) => rowsByPid.has(record.pid))
  const deadRecords = records.filter((record) => !rowsByPid.has(record.pid))

  return {
    records,
    aliveRecords,
    deadRecords,
    processRows,
    processGroupRows: processGroupRows.map((row) => ({ ...row, role: classifyProcess(row) })),
    summary: summarizeRows(processRows)
  }
}

export async function waitForCleanProcessState({
  ledgerPaths,
  pgid,
  timeoutMs = 10000,
  intervalMs = 250,
  readFile = readFileSync,
  readProcessTable = readSystemProcessTable
}) {
  const deadline = Date.now() + timeoutMs
  let lastCensus = null
  while (Date.now() <= deadline) {
    lastCensus = await collectProcessCensus({
      ledgerPaths,
      pgid,
      readFile,
      readProcessTable
    })
    if (lastCensus.records.length === 0 && lastCensus.processGroupRows.length === 0) {
      return lastCensus
    }
    await sleep(intervalMs)
  }

  const details = lastCensus ? formatCensus(lastCensus) : 'No process census was collected.'
  throw new Error(`Timed out waiting for clean process state.\n${details}`)
}

export async function waitForNoLiveProcessState({
  ledgerPaths,
  pgid,
  timeoutMs = 10000,
  intervalMs = 250,
  readFile = readFileSync,
  readProcessTable = readSystemProcessTable
}) {
  const deadline = Date.now() + timeoutMs
  let lastCensus = null
  while (Date.now() <= deadline) {
    lastCensus = await collectProcessCensus({
      ledgerPaths,
      pgid,
      readFile,
      readProcessTable
    })
    if (lastCensus.aliveRecords.length === 0 && lastCensus.processGroupRows.length === 0) {
      return lastCensus
    }
    await sleep(intervalMs)
  }

  const details = lastCensus ? formatCensus(lastCensus) : 'No process census was collected.'
  throw new Error(`Timed out waiting for live owned processes to exit.\n${details}`)
}

export async function pruneDeadOwnedProcessRecords({
  ledgerPaths,
  readFile = readFileSync,
  writeFile = writeFileSync,
  readProcessTable = readSystemProcessTable
}) {
  const processTable = await readProcessTable()
  const alivePids = new Set(processTable.map((row) => row.pid))
  const pruned = []
  for (const ledgerPath of ledgerPaths) {
    let parsed
    try {
      parsed = JSON.parse(readFile(ledgerPath, 'utf8'))
    } catch {
      continue
    }
    if (!Array.isArray(parsed)) {
      continue
    }
    const kept = []
    const removed = []
    for (const record of parsed) {
      if (!isOwnedProcessRecord(record) || !alivePids.has(record.pid)) {
        removed.push(record)
        continue
      }
      kept.push(record)
    }
    if (removed.length > 0) {
      writeFile(ledgerPath, `${JSON.stringify(kept, null, 2)}\n`)
      pruned.push({ ledgerPath, removed, kept })
    }
  }
  return pruned
}

export async function readSystemProcessTable({ platform = process.platform } = {}) {
  if (platform === 'win32') {
    const { stdout } = await execFileAsync(
      'powershell.exe',
      [
        '-NoProfile',
        '-ExecutionPolicy',
        'Bypass',
        '-Command',
        [
          'Get-CimInstance Win32_Process',
          'Select-Object ProcessId,ParentProcessId,WorkingSetSize,ExecutablePath,CommandLine',
          'ConvertTo-Json -Compress'
        ].join(' | ')
      ],
      { maxBuffer: 16 * 1024 * 1024 }
    )
    return parseWindowsProcessTable(stdout)
  }

  const { stdout } = await execFileAsync('ps', ['-axo', 'pid=,ppid=,pgid=,rss=,comm=,args='])
  return parseProcessTable(stdout)
}

export function parseProcessTable(stdout) {
  const rows = []
  for (const line of stdout.split('\n')) {
    const match = /^\s*(\d+)\s+(\d+)\s+(\d+)\s+(\d+)\s+(\S+)\s*(.*)$/.exec(line)
    if (!match) {
      continue
    }
    rows.push({
      pid: Number(match[1]),
      ppid: Number(match[2]),
      pgid: Number(match[3]),
      rssKb: Number(match[4]),
      command: match[5],
      args: match[6] ?? ''
    })
  }
  return rows
}

export function parseWindowsProcessTable(stdout) {
  const trimmed = typeof stdout === 'string' ? stdout.trim() : ''
  if (!trimmed) {
    return []
  }

  const parsed = JSON.parse(trimmed)
  const entries = Array.isArray(parsed) ? parsed : [parsed]
  return entries
    .map((entry) => windowsProcessRow(entry))
    .filter((row) => row !== null)
    .sort((a, b) => a.pid - b.pid)
}

export function classifyProcess(row) {
  const text = `${row.command} ${row.args}`
  const lowerText = text.toLowerCase()
  const commandName = (row.command.split(/[\\/]/).pop() ?? row.command).toLowerCase()
  if (commandName === 'cargo') {
    return 'cargo'
  }
  if (
    commandName === 'native_preview_host_helper' ||
    commandName === 'native_preview_host_helper.exe' ||
    lowerText.includes('native_preview_host_helper')
  ) {
    return 'native-preview-helper'
  }
  if (
    commandName === 'videorc-backend' ||
    commandName === 'videorc-backend.exe' ||
    lowerText.includes('videorc-backend')
  ) {
    return 'backend'
  }
  if (lowerText.includes('ffmpeg')) {
    return 'ffmpeg'
  }
  if (lowerText.includes('ffprobe')) {
    return 'ffprobe'
  }
  if (lowerText.includes('--type=renderer')) {
    return 'electron-renderer'
  }
  if (lowerText.includes('--type=gpu-process')) {
    return 'electron-gpu'
  }
  if (lowerText.includes('--type=')) {
    return 'electron-child'
  }
  if (
    /electron\.app\/contents\/macos\/electron/.test(lowerText) ||
    /\/videorc(?:\.app)?\//.test(lowerText) ||
    commandName === 'videorc.exe'
  ) {
    return 'electron-main'
  }
  if (
    lowerText.includes('pnpm') ||
    lowerText.includes('electron-vite') ||
    lowerText.includes('esbuild')
  ) {
    return 'tooling'
  }
  return 'other'
}

export function summarizeRows(rows) {
  const summary = {}
  for (const row of rows) {
    const role = row.role ?? classifyProcess(row)
    const entry = (summary[role] ??= { count: 0, rssKb: 0 })
    entry.count += 1
    entry.rssKb += row.rssKb
  }
  return summary
}

export function formatCensus(census) {
  const lines = []
  lines.push(`ledger records: ${census.records.length}`)
  for (const record of census.records) {
    const state = census.aliveRecords.some((alive) => alive.pid === record.pid) ? 'alive' : 'dead'
    lines.push(`  ${state} ${record.label}:${record.pid} from ${record.ledgerPath}`)
  }
  lines.push(`process group rows: ${census.processGroupRows.length}`)
  for (const row of census.processRows) {
    lines.push(
      `  pid=${row.pid} ppid=${row.ppid} pgid=${row.pgid} role=${row.role} rss=${row.rssKb}KB args=${row.args}`
    )
  }
  return lines.join('\n')
}

export function processExists(pid) {
  if (!Number.isInteger(pid) || pid <= 1) {
    return false
  }
  try {
    process.kill(pid, 0)
    return true
  } catch (error) {
    return error?.code === 'EPERM'
  }
}

export function ledgerExists(ledgerPath) {
  return existsSync(ledgerPath)
}

function isOwnedProcessRecord(value) {
  return (
    value &&
    typeof value === 'object' &&
    Number.isInteger(value.pid) &&
    value.pid > 1 &&
    typeof value.label === 'string' &&
    typeof value.startedAt === 'string'
  )
}

function windowsProcessRow(entry) {
  const pid = Number(entry?.ProcessId)
  if (!Number.isInteger(pid) || pid <= 0) {
    return null
  }
  const ppid = Number(entry?.ParentProcessId)
  const workingSetSize = Number(entry?.WorkingSetSize)
  const args = typeof entry?.CommandLine === 'string' ? entry.CommandLine : ''
  const command =
    typeof entry?.ExecutablePath === 'string' && entry.ExecutablePath.trim()
      ? entry.ExecutablePath
      : firstWindowsCommandToken(args)

  return {
    pid,
    ppid: Number.isInteger(ppid) && ppid >= 0 ? ppid : 0,
    pgid: null,
    rssKb: Number.isFinite(workingSetSize) ? Math.round(workingSetSize / 1024) : 0,
    command,
    args
  }
}

function firstWindowsCommandToken(commandLine) {
  if (typeof commandLine !== 'string') {
    return ''
  }
  const trimmed = commandLine.trim()
  if (!trimmed) {
    return ''
  }
  const quoted = /^"([^"]+)"/.exec(trimmed)
  if (quoted) {
    return quoted[1]
  }
  return trimmed.split(/\s+/)[0] ?? ''
}

function sleep(ms) {
  return new Promise((resolve) => setTimeout(resolve, ms))
}
