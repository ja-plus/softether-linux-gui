import { invoke } from '@tauri-apps/api/core'
import { listen } from '@tauri-apps/api/event'
import { ref } from 'vue'

export interface AccountSummary {
  name: string
  /** vpncmd 未给出状态时可能缺失，渲染层必须容错 */
  status?: string | null
  host: string
  port: number | null
  hub: string
  nic: string
}

export interface NicInfo {
  name: string
  enabled: boolean
  mac: string
}

export interface SessionSnapshot {
  name: string
  connected: boolean
  state: string
  host: string
  hub: string
  user: string
  serverVersion: string
  udpAccel: string
  encrypted: string
  tcpConnections: string
  connectedSince: number | null
  rxBytes: number
  txBytes: number
  rxPackets: number
  txPackets: number
}

export interface Overview {
  serviceRunning: boolean
  iface: string | null
  ifaceIpv4: string | null
  ifaceMtu: number | null
  accounts: AccountSummary[]
  nics: NicInfo[]
  snapshot: SessionSnapshot | null
  vpncmdReady: boolean
  vpnDir: string | null
  routes: string[]
  routeWarnings: string[]
}

export interface RouteConfig {
  subnets: string[]
  autoConnect: boolean
  dhcp: boolean
  mtu: number
}

export interface Settings {
  vpnDir: string | null
  vpncmdPath: string | null
  activeAccount: string | null
  notifications: boolean
  startMinimized: boolean
  diagTarget: string
  routes: RouteConfig
  /** 应用启动时自动启动 SoftEther 客户端 */
  startClientOnLaunch: boolean
  /** 应用退出时停止客户端（仅当客户端由本应用拉起） */
  stopClientOnExit: boolean
  /** 界面风格："win11" 或 "deepin"，其余取值按 win11 处理 */
  uiStyle: string
}

export type UiStyle = 'win11' | 'deepin'

/** 配置文件可能被手改成任意值，未知风格一律回落 win11 */
export function normalizeUiStyle(value: unknown): UiStyle {
  return value === 'deepin' ? 'deepin' : 'win11'
}

/** 把界面风格落到 <html data-style>，styles.css 据此切换整套设计 token */
export function applyUiStyle(value: unknown) {
  document.documentElement.dataset.style = normalizeUiStyle(value)
}

export interface AccountDetail {
  name: string
  host: string
  port: number
  hub: string
  user: string
  nic: string
  encrypted: boolean
  compress: boolean
  halfConnection: boolean
  noUdpAccel: boolean
  maxTcp: number
}

export interface AccountInput {
  name: string
  host: string
  port: number
  hub: string
  user: string
  nic: string
  password: string
  encrypted: boolean
  noUdpAccel: boolean
  compress: boolean
  maxTcp: number
}

export interface DiagResult {
  target: string
  sent: number
  received: number
  lossPercent: number
  rttAvgMs: number | null
  rttMinMs: number | null
  rttMaxMs: number | null
}

export const overview = ref<Overview | null>(null)
export const settings = ref<Settings | null>(null)
export const busy = ref<string | null>(null)
export const toast = ref<{ text: string; error: boolean } | null>(null)

let toastTimer: number | undefined

export function showToast(text: string, error = false) {
  toast.value = { text, error }
  window.clearTimeout(toastTimer)
  toastTimer = window.setTimeout(() => (toast.value = null), error ? 7000 : 3200)
}

export async function loadOverview() {
  overview.value = await invoke<Overview>('get_overview')
}

export async function loadSettings() {
  settings.value = await invoke<Settings>('get_settings')
  applyUiStyle(settings.value.uiStyle)
}

/**
 * 执行一次用户操作（连接/断开/启停服务/增删配置）。
 *
 * - 同一时刻只允许一个操作，避免两条 vpncmd 抢同一个客户端；
 *   被挡下的那次要给出提示，否则用户会以为按钮坏了。
 * - 命令本身已返回最新 `Overview` 时由调用方直接写回，不再多读一遍——
 *   多读一次会让按钮多置灰一整轮，体感像卡住。
 */
export async function run<T>(key: string, action: () => Promise<T>): Promise<T | null> {
  if (busy.value) {
    showToast('上一个操作还没完成，请稍候')
    return null
  }
  busy.value = key
  try {
    const result = await action()
    // 这类命令的返回值就是执行后的完整快照，直接写回，省掉一轮多余的读取
    if (isOverview(result)) overview.value = result
    return result
  } catch (error) {
    showToast(String(error), true)
    return null
  } finally {
    busy.value = null
  }
}

function isOverview(value: unknown): value is Overview {
  return !!value && typeof value === 'object' && 'serviceRunning' in value && 'accounts' in value
}

export function connectAccount(name: string) {
  return run(`connect:${name}`, async () => {
    const result = await invoke<Overview>('connect', { name })
    showToast(`${name} 已连接`)
    return result
  })
}

export function disconnectAccount(name: string) {
  return run(`disconnect:${name}`, async () => {
    const result = await invoke<Overview>('disconnect', { name })
    showToast(`${name} 已断开`)
    return result
  })
}

export function startService() {
  return run('service', async () => {
    const result = await invoke<Overview>('start_service')
    showToast('VPN 服务已启动')
    return result
  })
}

export function stopService() {
  return run('service', async () => {
    const result = await invoke<Overview>('stop_service')
    showToast('VPN 服务已停止')
    return result
  })
}

/** 当前运行的客户端是否由本应用拉起 */
export function clientManaged() {
  return invoke<boolean>('client_managed')
}

/**
 * 静默刷新状态。
 *
 * 刻意**不走 `run()`**：轮询是背景行为，不该占用 `busy`。占用的后果是
 * 每 2 秒全界面按钮同步闪一次灰，且窗口期内用户点「连接」会被
 * `run()` 里的 `if (busy.value) return null` 静默丢掉。
 *
 * 状态写入只走命令返回值这一路：后端的 `refresh` 刻意不再 emit "overview"，
 * 两条路都推同一份数据会让界面每 2 秒多渲染一遍；tick 里的 recordRate
 * 也依赖「await 之后 overview 必然已是新值」这个时序。
 */
export async function refresh() {
  overview.value = await invoke<Overview>('refresh')
  return overview.value
}

export async function bootstrap() {
  // 首屏数据各自失败互不牵连：任一命令失败只让对应区块降级，不能阻断整个界面渲染
  await Promise.allSettled([loadOverview(), loadSettings()])
  // 事件订阅失败不影响已有数据，页面照常可用（只是失去推送更新）
  await Promise.allSettled([
    listen<Overview>('overview', (event) => {
      overview.value = event.payload
    }),
    listen<string>('action-error', (event) => showToast(event.payload, true)),
  ])
}

export function formatBytes(value: number): string {
  if (!Number.isFinite(value) || value <= 0) return '0 B'
  const units = ['B', 'KB', 'MB', 'GB', 'TB']
  let size = value
  let index = 0
  while (size >= 1024 && index < units.length - 1) {
    size /= 1024
    index += 1
  }
  return `${size >= 100 || index === 0 ? size.toFixed(0) : size.toFixed(1)} ${units[index]}`
}

export function formatRate(bytesPerSecond: number): string {
  return `${formatBytes(bytesPerSecond)}/s`
}

export function formatDuration(seconds: number): string {
  if (!Number.isFinite(seconds) || seconds < 0) return '—'
  const hours = Math.floor(seconds / 3600)
  const minutes = Math.floor((seconds % 3600) / 60)
  const secs = Math.floor(seconds % 60)
  if (hours > 0) return `${hours} 小时 ${minutes} 分`
  if (minutes > 0) return `${minutes} 分 ${secs} 秒`
  return `${secs} 秒`
}
