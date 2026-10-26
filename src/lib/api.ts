import { invoke } from '@tauri-apps/api/core'
import { listen } from '@tauri-apps/api/event'
import { ref } from 'vue'
import { applyLocale, t } from './i18n'

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
  /**
   * 虚拟网卡累计收发字节（后端读 sysfs）。
   *
   * 实时速率必须用这两个值算，不要用 `snapshot.rxBytes`：后者来自
   * `AccountStatusGet`，后端为它缓存了几秒，2 秒一轮的速率刷新会一直是台阶状。
   * 累计量展示仍用 `snapshot.rxBytes`（会话层语义）。
   */
  linkRxBytes: number
  linkTxBytes: number
}

export interface RouteConfig {
  subnets: string[]
  autoConnect: boolean
  dhcp: boolean
  mtu: number
}

/** 客户端目录的来源，对应设置页的三个选项 */
export type ClientSource = 'bundled' | 'detected' | 'custom'

export interface ClientCandidate {
  /** 目录绝对路径 */
  dir: string
  source: ClientSource
  /** 必需文件是否齐备（vpnclient + vpncmd + hamcore.se2） */
  usable: boolean
  /** 不可用时的具体缺项，直接展示给用户 */
  problem: string | null
  /** 客户端版本，取不到为 null */
  version: string | null
  /** 目录可写：不可写时客户端写不了自己的配置与日志 */
  writable: boolean
}

export interface Settings {
  vpnDir: string | null
  /** 客户端来源；null 表示老配置，按 builtin 处理 */
  clientSource: ClientSource | null
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
  /** 界面语言："zh" 或 "en"，其余取值按 zh 处理 */
  locale: string
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
  applyLocale(settings.value.locale)
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
    showToast(t('common.busy'))
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
    showToast(t('app.toastConnected', { name }))
    return result
  })
}

export function disconnectAccount(name: string) {
  return run(`disconnect:${name}`, async () => {
    const result = await invoke<Overview>('disconnect', { name })
    showToast(t('app.toastDisconnected', { name }))
    return result
  })
}

export function startService() {
  return run('service', async () => {
    const result = await invoke<Overview>('start_service')
    showToast(t('app.toastServiceStarted'))
    return result
  })
}

export function stopService() {
  return run('service', async () => {
    const result = await invoke<Overview>('stop_service')
    showToast(t('app.toastServiceStopped'))
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
  if (hours > 0) return t('duration.hoursMinutes', { h: hours, m: minutes })
  if (minutes > 0) return t('duration.minutesSeconds', { m: minutes, s: secs })
  return t('duration.seconds', { s: secs })
}

/** vpncmd 的输出语言跟随客户端 lang.config，状态值可能是中文或英文，显示前统一翻译 */
export function localizeStatus(value?: string | null): string {
  if (!value) return t('common.unknown')
  // 先排除否定状态：`Not Connected` 里也含有 `Connected`
  if (value.includes('未连接') || value.toLowerCase().includes('not connected')) {
    return t('status.disconnected')
  }
  const known: [string, Parameters<typeof t>[0]][] = [
    ['连接完成', 'status.completed'],
    ['Connection Completed', 'status.completed'],
    ['已连接', 'status.connected'],
    ['Connected', 'status.connected'],
    ['连接中', 'status.connecting'],
    ['Connecting', 'status.connecting'],
  ]
  for (const [needle, key] of known) {
    if (value.includes(needle)) return t(key)
  }
  return value
}

/** 账户是否处于已连接状态。中英文输出都要认，且不能把“未连接”误判成已连接。 */
export function isAccountConnected(status?: string | null): boolean {
  const value = status ?? ''
  if (value.includes('未连接') || value.toLowerCase().includes('not connected')) return false
  return ['已连接', '连接完成', '连接中', 'Connected', 'Connection Completed', 'Connecting'].some((needle) =>
    value.includes(needle),
  )
}

/** vpncmd 的布尔值随语言变化：中文“是/否”、英文 Yes/No */
export function isYes(value?: string | null): boolean {
  return value === '是' || /^yes$/i.test(value?.trim() ?? '')
}

/** 加密等开关字段可能带后缀（如 “已启用 (算法: TLS)”), 只翻译前缀 */
export function localizeFlag(value?: string | null): string {
  const text = value ?? ''
  if (text.startsWith('已启用')) return t('common.enabled') + text.slice(3)
  if (text.startsWith('已禁用')) return t('common.disabled') + text.slice(3)
  return text
}
