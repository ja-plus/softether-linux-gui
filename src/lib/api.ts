import { invoke } from '@tauri-apps/api/core'
import { listen } from '@tauri-apps/api/event'
import { ref } from 'vue'
import { Message } from '@arco-design/web-vue'
import '@arco-design/web-vue/es/message/style/css.js'
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
  /** 正在跑的那个客户端进程所在的目录；没跑时为 null */
  runningDir: string | null
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
  /** 界面风格："win11" / "deepin" / "mac"，其余取值按 win11 处理 */
  uiStyle: string
  /** 界面语言："zh" 或 "en"，其余取值按 zh 处理 */
  locale: string
}

export type UiStyle = 'win11' | 'deepin' | 'mac'

/** 已知风格的白名单；新增风格只改这一处，归一化和渲染都跟着走 */
const UI_STYLES: readonly UiStyle[] = ['win11', 'deepin', 'mac']

/** 配置文件可能被手改成任意值，未知风格一律回落 win11 */
export function normalizeUiStyle(value: unknown): UiStyle {
  return UI_STYLES.includes(value as UiStyle) ? (value as UiStyle) : 'win11'
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

/** 某个客户端目录里有多少连接账号；账号是跟着客户端目录走的 */
export interface AccountSource {
  dir: string
  accounts: number
  current: boolean
  /** 这个目录的来源；切过去时要原样回传，写错会让内置客户端被记成「自己指定」 */
  source: ClientSource
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
/**
 * 正在执行的那次操作到底在干什么（如"正在停止客户端"）。
 *
 * `busy` 只是个互斥标记，用户看不到它；授权框弹出、客户端收尾这些环节又能
 * 静默卡上十几秒，界面上要是只有一颗灰按钮，用户就会认定"点了没反应"。
 * 所以把描述一并暴露出去，按钮上直接显示。
 */
export const busyText = ref<string | null>(null)

/** 轻提示走 Arco 的 Message。注意 duration 是毫秒（源码 setTimeout 直接用），
    不是文档写的秒：错误停 7 秒、普通提示 3.2 秒，和原 toast 时长一致。 */
export function showToast(text: string, error = false) {
  if (error) {
    Message.error({ content: text, duration: 7000 })
  } else {
    Message.success({ content: text, duration: 3200 })
  }
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
export async function run<T>(
  key: string,
  action: () => Promise<T>,
  label?: string,
): Promise<T | null> {
  if (busy.value) {
    showToast(t('common.busy'))
    return null
  }
  busy.value = key
  busyText.value = label ?? null
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
    busyText.value = null
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

/**
 * 启停服务的最长等待时间。
 *
 * 后端会一直等到客户端进程真的消失才返回，中间还要走授权、SIGTERM 收尾，
 * 正常 1 秒内、最坏十几秒。超过这个上限只可能是真卡住了，必须把 loading
 * 收掉并明说，否则界面会一直转圈、按钮永远点不动。
 */
const SERVICE_TIMEOUT_MS = 45_000

/** 给一次操作加硬上限；超时按失败处理，不能让界面无限转圈 */
function withTimeout<T>(task: Promise<T>, message: string): Promise<T> {
  return new Promise<T>((resolve, reject) => {
    const timer = window.setTimeout(() => reject(new Error(message)), SERVICE_TIMEOUT_MS)
    task.then(
      (value) => {
        window.clearTimeout(timer)
        resolve(value)
      },
      (error) => {
        window.clearTimeout(timer)
        reject(error)
      },
    )
  })
}

export function startService() {
  return run(
    'service',
    async () => {
      const result = await withTimeout(
        invoke<Overview>('start_service'),
        t('overview.startTimeout'),
      )
      showToast(t('app.toastServiceStarted'))
      return result
    },
    t('overview.startTakesLong'),
  )
}

export function stopService() {
  return run(
    'service',
    async () => {
      // 不加中途提示：loading 与横幅已经把"正在做"表达清楚了，
      // 再弹一条"正在停止"的 toast 只是噪音。
      const result = await withTimeout(
        invoke<Overview>('stop_service'),
        t('overview.stopTimeout'),
      )
      showToast(t('app.toastServiceStopped'))
      return result
    },
    t('overview.stopTakesLong'),
  )
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
/**
 * 每一轮状态请求的序号。
 *
 * 手动刷新和 2 秒轮询会并发：手动那次要**丢掉缓存重读**（慢，几百毫秒），
 * 轮询那次吃缓存（快）。谁慢谁后到，如果不加序号，慢的那份会把新数据盖回去。
 * 只认"最后发起的那次"，写入就一定是最新意图。
 */
let refreshSeq = 0

export async function refresh(fresh = false) {
  const seq = ++refreshSeq
  const result = await invoke<Overview>('refresh', { fresh })
  if (seq === refreshSeq) overview.value = result
  return result
}

/** 从 vpn_client.config 静态读出来的配置（不经过运行中的客户端） */
export interface StaticAccount {
  name: string
  host: string
  port: number
  hub: string
  nic: string
  /** 是否设过密码。匿名认证的配置连不上 */
  hasPassword: boolean
}

/** 一个客户端目录 + 它里面的全部配置 */
export interface ClientAccountGroup {
  dir: string
  label: string
  current: boolean
  /** 这个目录的来源；「切换并连接」要原样回传，写错会让内置客户端被记成「自己指定」 */
  source: ClientSource
  accounts: StaticAccount[]
}

/** 列出所有客户端目录里的配置，界面据此标注"哪一份正在使用" */
export function clientAccountGroups() {
  return invoke<ClientAccountGroup[]>('client_account_groups')
}

/**
 * 在另一个客户端目录里连接某个配置：切客户端 → 启动 → 连接，后端一次做完。
 *
 * 前端只负责先问一句——切过去会断掉当前隧道。
 */
export function connectInClient(dir: string, name: string, source?: ClientSource) {
  return run(`connect:${name}`, async () => {
    const result = await withTimeout(
      invoke<Overview>('connect_in_client', { dir, name, source }),
      t('overview.connectTimeout'),
    )
    showToast(t('app.toastConnected', { name }))
    return result
  })
}

/** 各客户端目录各自的账号数。用于回答"我的配置去哪了" */
export function accountSources() {
  return invoke<AccountSource[]>('account_sources')
}

/**
 * 当前客户端里没有账号时，找出哪个客户端目录里有。
 *
 * 连接页和连接配置页都要展示"账号在别处"，判定逻辑放在这里共用一份，
 * 免得两边哪天对"什么叫有账号"产生分歧。
 */
export async function findSourceWithAccounts(): Promise<AccountSource | null> {
  const sources = await accountSources()
  return sources.find((item) => !item.current && item.accounts > 0) ?? null
}

/**
 * 切到另一个客户端目录。后端会把"停旧的 → 写设置 → 起新的"整套做完，
 * 所以这里跟启停服务共用一个忙碌标记，避免两个动作交叉。
 */
export function switchClientDir(dir: string, source?: ClientSource) {
  return run(
    'service',
    async () => {
      const result = await withTimeout(
        invoke<Overview>('switch_client_dir', { dir, source }),
        t('overview.switchTimeout'),
      )
      showToast(t('overview.switchedClient'))
      return result
    },
    t('overview.switchingClient'),
  )
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

/** 状态串是不是"没连上"。必须先排除：`Connected` 会命中 `Not Connected` */
function isAccountDisconnected(value: string): boolean {
  return value.includes('未连接') || value.includes('离线') || value.toLowerCase().includes('not connected')
}

/**
 * 账户是否**真的**连上了。
 *
 * 「连接中」不算——客户端连不上服务器时会无限重连，AccountList 的状态一直停在
 * 「连接中」。把它当已连接，界面就会显示绿色「已连接」而实际一个包都过不去
 * （踩过：账户密码不对，会话卡在「重试」40 秒以上，配置行却是绿的）。
 */
export function isAccountConnected(status?: string | null): boolean {
  const value = status ?? ''
  if (isAccountDisconnected(value)) return false
  return ['已连接', '连接完成', 'Connected', 'Connection Completed'].some((needle) => value.includes(needle))
}

/** 是否"正在连、还没通"：握手 / 认证 / 反复重连。界面上既不能说已连接也不该说未连接 */
export function isAccountConnecting(status?: string | null): boolean {
  const value = status ?? ''
  if (!value || isAccountDisconnected(value) || isAccountConnected(value)) return false
  return ['连接中', '正在连接', '开始连接', '重试', '验证用户身份', 'Connecting', 'Retrying', 'Authenticating'].some(
    (needle) => value.includes(needle),
  )
}

/** vpncmd 的布尔值随语言变化：中文“是/否”、英文 Yes/No */
export function isYes(value?: string | null): boolean {
  return value === '是' || /^yes$/i.test(value?.trim() ?? '')
}

/** 加密等开关字段可能带后缀（如 “已启用 (算法: TLS)”), 只翻译前缀 */
export function localizeFlag(value?: string | null): string {
  const text = value ?? ''
  const localizeSuffix = (suffix: string) => suffix.replace(/算法\s*:/g, `${t('overview.algorithm')}:`)
  if (text.startsWith('已启用')) return t('common.enabled') + localizeSuffix(text.slice(3))
  if (text.startsWith('已禁用')) return t('common.disabled') + localizeSuffix(text.slice(3))
  return text
}
