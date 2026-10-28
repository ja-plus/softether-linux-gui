<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref, watch } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import { ArrowRightLeft, Loader2, Plus, RefreshCw, PlugZap, Power } from 'lucide-vue-next'
import DisconnectIcon from './DisconnectIcon.vue'
import { confirmDialog } from '../lib/dialog'
import SelectInput, { type SelectOption } from './SelectInput.vue'
import Sparkline from './Sparkline.vue'
import {
  busy,
  busyText,
  connectAccount,
  disconnectAccount,
  findSourceWithAccounts,
  formatBytes,
  formatRate,
  isAccountConnecting,
  isYes,
  localizeFlag,
  overview,
  refresh,
  settings,
  showToast,
  startService,
  switchClientDir,
  type AccountSource,
  type NicInfo,
} from '../lib/api'
import { t, locale } from '../lib/i18n'

interface RateSample {
  rx: number
  tx: number
}

const rates = ref<RateSample[]>([])
const errorText = ref('')
const now = ref(Date.now())
/** 手动刷新的忙碌态，与轮询、与操作类 busy 都分开 */
const refreshing = ref(false)

let pollTimer: number | undefined
let clockTimer: number | undefined
let previous: { at: number; rx: number; tx: number } | null = null
/** 轮询防重入：vpncmd 读状态偶尔要 1s 以上，setInterval 会叠请求并让旧响应盖掉新的 */
let polling = false

const snapshot = computed(() => overview.value?.snapshot ?? null)
/** 启停服务进行中。两个按钮共用同一份忙碌标记，图标转圈即可 */
const serviceBusy = computed(() => busy.value === 'service')

/**
 * 圆按钮为什么点不了。空串表示可以点。
 *
 * 按钮置灰时必须说清原因，否则用户只知道"点不动"，不知道该去启动服务
 * 还是先去建配置。
 */
const connectBlockedReason = computed(() => {
  if (!overview.value) return ''
  if (!overview.value.serviceRunning) return t('overview.connectNeedsService')
  if (!accounts.value.length) return t('overview.connectNeedsProfile')
  if (!selectedName.value) return t('overview.connectNeedsSelection')
  return ''
})

/**
 * 置灰用的内联样式。
 *
 * 写成内联而不是只靠 `:disabled` 选择器：主题（win11/deepin/mac）里可能有
 * 更具体的选择器重新给背景上色，光靠样式表会被悄悄压掉——按钮看着还是能点。
 * 内联样式是最高的那一层，主题换色也压不动它。
 */
const blockedStyle = computed(() =>
  connectBlockedReason.value
    ? {
        background: 'linear-gradient(145deg, var(--line-strong), var(--line))',
        color: 'var(--text-soft)',
        boxShadow: 'none',
        cursor: 'default',
      }
    : undefined,
)

/**
 * 另一个客户端目录里找到的账号（仅在没有可选账号时才去查）。
 *
 * 账号是跟着客户端目录走的：用户把来源切到内置之后，自己装的那份里的
 * 配置不会跟过来。这里把"账号在哪"直接摆到界面上，并给一键切过去。
 */
const otherSource = ref<AccountSource | null>(null)

/**
 * 点击切换。服务在跑的时候先问一句再动手——切换会停掉当前服务，
 * 正在连着的隧道会断，这个动作不能默默发生。
 *
 * 来源要按那份客户端的实际情况回传：不传的话后端只能猜，
 * 猜成"自己指定"就会把内置客户端的来源写错。
 */
async function askSwitch(target: AccountSource) {
  if (overview.value?.serviceRunning) {
    const ok = await confirmDialog({
      title: t('overview.switchTitle'),
      body: t('overview.switchBody'),
      confirmText: t('overview.switchConfirm'),
      danger: true,
    })
    if (!ok) return
  }
  void switchClientDir(target.dir, target.source)
}

async function loadOtherSource() {
  // 不看服务是否在跑：停了之后仍要能告诉用户"账号在哪个目录里"，
  // 那正是切换的唯一时机
  if (accounts.value.length) {
    otherSource.value = null
    return
  }
  try {
    otherSource.value = await findSourceWithAccounts()
  } catch {
    // 拿不到就退回纯文案提示，不足以打断页面
    otherSource.value = null
  }
}

// 声明必须在下面 watch 之前：watch 的 getter 会立刻执行一次，
// 先跑 watch 再声明 accounts 会撞上 TDZ，整个组件首帧就抛错白屏
const accounts = computed(() => overview.value?.accounts ?? [])

// immediate：首屏就可能是"服务在跑 + 没有账号"，那时没有任何状态变化会
// 触发上面这个 watch，不主动查一次就一直空着
watch(
  () => [overview.value?.serviceRunning, accounts.value.length] as const,
  () => void loadOtherSource(),
  { immediate: true },
)

const nics = computed(() => overview.value?.nics ?? [])

/**
 * 客户端跑着、但一块虚拟网卡都没有。
 *
 * 为什么必须带上 `serviceRunning`：后端只在 `vpncmd_ready && running` 时才读
 * 清单（`vpn.rs:304`），停服期间 nics 恒为空。少了这个条件，停服那一刻会
 * 冒出"没有网卡"的引导，用户点了也建不成——vpncmd 根本没有客户端可连。
 */
const noNic = computed(
  () => !!overview.value?.serviceRunning && !!overview.value?.vpncmdReady && nics.value.length === 0,
)

/** 建网卡时的默认名。这是系统接口名，不跟随界面语言。 */
const DEFAULT_NIC_NAME = 'VPN'
const creatingNic = ref(false)

async function createDefaultNic() {
  if (creatingNic.value) return
  creatingNic.value = true
  try {
    const list = await invoke<NicInfo[]>('create_nic', { name: DEFAULT_NIC_NAME })
    // 只换 nics 字段：overview 里还有服务状态、路由等，整体覆盖会闪一下
    if (overview.value) overview.value = { ...overview.value, nics: list }
    showToast(t('nic.created', { name: DEFAULT_NIC_NAME }))
  } catch (error) {
    showToast(String(error), true)
  } finally {
    creatingNic.value = false
  }
}

const currentRate = computed(() => rates.value[rates.value.length - 1] ?? { rx: 0, tx: 0 })
const elapsed = computed(() => {
  const since = snapshot.value?.connectedSince
  if (!since) return null
  return Math.max(0, now.value / 1000 - since)
})
const elapsedClock = computed(() => {
  if (elapsed.value === null) return null
  const totalSeconds = Math.floor(elapsed.value)
  const hours = Math.floor(totalSeconds / 3600)
  const minutes = Math.floor((totalSeconds % 3600) / 60)
  const seconds = totalSeconds % 60
  return [hours, minutes, seconds].map((value) => String(value).padStart(2, '0')).join(':')
})

/** 连接页只负责挑一个配置连上；增删改统一放到「连接配置」页 */
const selectedName = ref('')

const accountOptions = computed<SelectOption<string>[]>(() =>
  accounts.value.map((account) => ({
    label: account.name,
    value: account.name,
    hint: `${account.host}:${account.port ?? 443}`,
    tag: settings.value?.activeAccount === account.name ? t('profiles.default') : undefined,
  })),
)

// 默认选中设置里的默认配置，否则第一条；配置列表变化后兜住已失效的名字
watch(
  [accounts, () => settings.value?.activeAccount],
  () => {
    if (accounts.value.some((account) => account.name === selectedName.value)) return
    const preferred = settings.value?.activeAccount
    selectedName.value = accounts.value.some((account) => account.name === preferred)
      ? (preferred as string)
      : (accounts.value[0]?.name ?? '')
  },
  { immediate: true },
)

/**
 * 正在连接中的配置名（优先选中的那条），空串表示没有。
 *
 * 「连接中」时圆按钮必须是「断开」：客户端不许对同一配置重复 AccountConnect
 * （错误码 35），点「连接」只会收到一句报错，想重试就先断开——
 * 和「连接配置」页同一条规则，两处不能一个能断、一个不能断。
 */
const connectingName = computed(() => {
  const selected = accounts.value.find((account) => account.name === selectedName.value)
  if (selected && isAccountConnecting(selected.status)) return selected.name
  return accounts.value.find((account) => isAccountConnecting(account.status))?.name ?? ''
})

// 断开后让选择器停在刚断开的那条，方便立刻重连
watch(
  () => snapshot.value?.name,
  (name) => {
    if (name) selectedName.value = name
  },
)

/** 后端返回多条路由告警，分隔符也要跟随语言 */
function joinWarnings(list: string[]): string {
  return list.join(locale.value === 'en' ? '; ' : '；')
}

function recordRate() {
  const current = overview.value
  // 未连接时把曲线清空；连接状态本身由 snapshot 决定，速率取网卡计数器
  if (!current?.snapshot) {
    rates.value = []
    previous = null
    return
  }
  const at = Date.now()
  if (previous) {
    const deltaSeconds = (at - previous.at) / 1000
    if (deltaSeconds > 0.2) {
      // 网卡重建（重连）时计数器会归零，Math.max 兜住这种负增量
      const rx = Math.max(0, (current.linkRxBytes - previous.rx) / deltaSeconds)
      const tx = Math.max(0, (current.linkTxBytes - previous.tx) / deltaSeconds)
      rates.value = [...rates.value, { rx, tx }].slice(-40)
    }
  }
  previous = { at, rx: current.linkRxBytes, tx: current.linkTxBytes }
}

async function tick() {
  if (polling) return
  polling = true
  try {
    await refresh()
    errorText.value = ''
  } catch (error) {
    errorText.value = String(error)
  } finally {
    polling = false
  }
  recordRate()
}

/**
 * 手动刷新：丢掉缓存重读一份状态。
 *
 * 三个坑一次填掉：
 * 1. 不能调 `tick()`——它开头有 `if (polling) return`，2 秒一轮的自动轮询
 *    正好在跑时，点击会被静默吞掉，表现就是"点了没反应"。
 * 2. 必须传 `fresh`：不传的话后端读的是缓存（清单 30 秒、会话 5 秒），
 *    拿回同一份数据，看上去仍然什么都没发生。
 * 3. 转圈要有最短时长：重读在缓存命中时只要几毫秒，图标闪一下等于没反馈。
 */
const MIN_SPIN_MS = 450

async function manualRefresh() {
  if (refreshing.value) return
  refreshing.value = true
  const startedAt = Date.now()
  try {
    await refresh(true)
    errorText.value = ''
    recordRate()
    const elapsed = Date.now() - startedAt
    if (elapsed < MIN_SPIN_MS) {
      await new Promise((resolve) => window.setTimeout(resolve, MIN_SPIN_MS - elapsed))
    }
    showToast(t('overview.refreshed'))
  } catch (error) {
    errorText.value = String(error)
  } finally {
    refreshing.value = false
  }
}

/**
 * 轮询节奏。
 *
 * 窗口可见时 2 秒一次（速率曲线要有肉），窗口收起/最小化时 10 秒一次：
 * 那会儿没人看曲线，但托盘状态还得跟得上。原来不分前后台一律 2 秒，
 * 常驻托盘时也在白白唤醒后端。
 */
const POLL_VISIBLE_MS = 2000
const POLL_HIDDEN_MS = 10000

function schedulePoll() {
  window.clearInterval(pollTimer)
  pollTimer = window.setInterval(tick, document.hidden ? POLL_HIDDEN_MS : POLL_VISIBLE_MS)
}

function onVisibilityChange() {
  schedulePoll()
  // 切回前台立刻补一次，别让用户对着 10 秒前的数据发呆
  if (!document.hidden) tick()
}

onMounted(() => {
  schedulePoll()
  // 连接时长只在已连接时有意义，未连接时不必每秒改动一次 DOM
  clockTimer = window.setInterval(() => {
    if (snapshot.value) now.value = Date.now()
  }, 1000)
  document.addEventListener('visibilitychange', onVisibilityChange)
  tick()
})

onBeforeUnmount(() => {
  window.clearInterval(pollTimer)
  window.clearInterval(clockTimer)
  document.removeEventListener('visibilitychange', onVisibilityChange)
})

watch(
  () => snapshot.value?.name,
  () => {
    rates.value = []
    previous = null
  },
)
</script>

<template>
  <section class="overview">
    <header class="head">
      <div>
        <h1>{{ t('overview.title') }}</h1>
      </div>
      <div class="row">
        <!-- 启停客户端的按钮统一收在「SoftEther 客户端」页：
             那是它真正作用的对象，放在这里会让人以为它只管当前这条隧道 -->
        <button
          class="ghost refresh-icon"
          :disabled="refreshing"
          :aria-label="t('overview.refresh')"
          :title="t('overview.refreshTooltip')"
          @click="manualRefresh"
        >
          <RefreshCw :size="16" :class="{ spin: refreshing }" />
        </button>
      </div>
    </header>

    <div v-if="serviceBusy && busyText" class="banner busy">
      <Loader2 :size="13" class="spin" />
      <span>{{ busyText }}</span>
      <span class="hint">{{ t('overview.busyHint') }}</span>
    </div>

    <div v-if="errorText" class="banner bad">
      {{ t('overview.readError', { error: errorText }) }}
      <span class="hint">{{ t('overview.readErrorHint') }}</span>
    </div>

    <div v-if="!overview?.vpncmdReady" class="banner warn">
      {{ t('overview.vpncmdMissingPrefix') }}<span class="mono">vpncmd</span>{{ t('overview.vpncmdMissingSuffix') }}
    </div>

    <div v-if="overview && !overview.serviceRunning" class="banner off">
      <span>{{ t('overview.serviceDown') }}</span>
      <button class="success" :disabled="!!busy" @click="startService">
        <Loader2 v-if="serviceBusy" :size="14" class="spin" />
        <Power v-else :size="14" />
        {{ t('overview.startService') }}
      </button>
    </div>

    <div class="status card" :class="{ connected: !!snapshot }">
      <div class="status-main">
        <template v-if="snapshot">
          <div class="status-heading">
            <strong class="strong">{{ snapshot.name }}</strong>
            <span class="badge ok"><i class="dot" />{{ t('status.connected') }}</span>
          </div>
          <div class="meta">
            <span>{{ t('overview.server') }} <span class="mono">{{ snapshot.host }}</span></span>
            <span>{{ t('overview.hub') }} <span class="mono">{{ snapshot.hub || '—' }}</span></span>
          </div>
        </template>
        <template v-else>
          <div class="status-heading">
            <!-- 客户端在反复重连时 AccountList 一直报「连接中」，这里要如实说"连接中"，
                 不能跟着 snapshot 缺失就画成"未连接"——那样和配置页的绿色已连接会打架 -->
            <span v-if="connectingName" class="badge warn">
              <i class="dot" />{{ t('status.connecting') }}
            </span>
            <span v-else class="badge off">
              <i class="dot" />{{ overview?.serviceRunning ? t('status.disconnected') : t('status.serviceDown') }}
            </span>
          </div>
          <div v-if="accounts.length" class="picker">
            <SelectInput
              v-model="selectedName"
              :options="accountOptions"
              :disabled="!overview?.serviceRunning"
            />
          </div>
          <!-- 账号为空有三种完全不同的原因，必须分开说：
               ① 客户端起来了但一块网卡都没有 → 先建网卡，否则新建配置时无处可绑
               ② 客户端空着（内置那份是全新空配置）→ 账号可能躺在别的客户端目录里
               ③ 客户端里确实没有 → 去「连接配置」新建
               只写一句"暂无连接配置"，用户只会以为应用坏了。 -->
          <div v-else class="no-accounts">
            <template v-if="noNic">
              <span class="muted">{{ t('overview.noNics') }}</span>
              <span class="hint">{{ t('overview.noNicsHint') }}</span>
              <button
                class="primary small"
                :disabled="!!busy || creatingNic"
                @click="createDefaultNic"
              >
                <Loader2 v-if="creatingNic" :size="13" class="spin" />
                <Plus v-else :size="13" />
                {{ creatingNic ? t('overview.creatingNic') : t('overview.createNic') }}
              </button>
            </template>
            <template v-else>
              <span class="muted">{{ t('overview.noAccounts') }}</span>
              <span v-if="!otherSource" class="hint">{{ t('overview.noAccountsHint') }}</span>
            </template>
            <!-- 账号躺在别的客户端目录：和有没有网卡无关，单独一条 -->
            <template v-if="otherSource">
              <span class="muted">
                {{ t('overview.accountsElsewhere', { dir: otherSource.dir, n: otherSource.accounts }) }}
              </span>
              <!-- 一键切换：运行时先弹一次确认（会停掉当前服务），
                   确认后由后端把"停 → 切 → 起"一次做完，用户不用管中间步骤 -->
              <button
                class="success small"
                :disabled="!!busy"
                @click="askSwitch(otherSource)"
              >
                <Loader2 v-if="serviceBusy" :size="13" class="spin" />
                <ArrowRightLeft v-else :size="13" />
                {{ t('overview.switchTo') }}
              </button>
            </template>
          </div>
        </template>
      </div>

      <div class="status-actions">
        <button
          v-if="snapshot"
          class="connection-action disconnect-action"
          :class="{ 'showing-duration': elapsedClock !== null && busy !== `disconnect:${snapshot.name}` }"
          :disabled="!!busy"
          @click="disconnectAccount(snapshot.name)"
        >
          <DisconnectIcon class="disconnect-icon" />
          <span v-if="elapsedClock !== null" class="action-duration">{{ elapsedClock }}</span>
          <span class="action-label">{{ busy === `disconnect:${snapshot.name}` ? t('common.disconnecting') : t('common.disconnect') }}</span>
        </button>
        <!-- 「连接中」也是「断开」：客户端不许对同一配置重复 AccountConnect（错误码 35），
             点「连接」只会收到报错；想重试就先断开——和「连接配置」页同一条规则 -->
        <button
          v-else-if="connectingName"
          class="connection-action disconnect-action"
          :disabled="!!busy"
          @click="disconnectAccount(connectingName)"
        >
          <DisconnectIcon class="disconnect-icon" />
          <span class="action-label">
            {{
              busy === `connect:${connectingName}`
                ? t('common.connecting')
                : busy === `disconnect:${connectingName}`
                  ? t('common.disconnecting')
                  : t('common.disconnect')
            }}
          </span>
        </button>
        <!-- 永远渲染这颗圆按钮：没有配置/服务没起时置灰。
             直接不显示会让状态卡右半边突然空掉，用户还得自己猜"连接去哪了"。 -->
        <button
          v-else
          class="connection-action connect-action"
          :disabled="!!busy || !!connectBlockedReason"
          :title="connectBlockedReason || undefined"
          :style="blockedStyle"
          @click="connectAccount(selectedName)"
        >
          <svg class="connect-icon" viewBox="0 0 1024 1024" width="34" height="34" fill="currentColor" aria-hidden="true">
            <path d="M935.754805 89.966493a47.62257 47.62257 0 0 0-3.672102-4.590127A252.170113 252.170113 0 0 0 753.698885 8.893371c-66.384715 1.147532-131.794027 24.72931-180.621505 72.35188l-175.055976 175.572365A248.096375 248.096375 0 0 0 325.956407 437.209615a258.538914 258.538914 0 0 0 76.540371 179.875609c1.204908 1.204908 2.52457 2.295064 3.901608 3.270466a68.163389 68.163389 0 0 0 94.72875-4.475374 68.278142 68.278142 0 0 0-2.409817-95.991035 123.187539 123.187539 0 0 1-36.548887-85.720625 112.22861 112.22861 0 0 1 32.30302-81.302628l174.654339-175.342859c22.72113-21.745728 52.384827-34.425954 83.884575-32.589903a116.93349 116.93349 0 0 1 82.851796 36.835771c22.950636 22.72113 35.917745 53.130722 36.548887 85.778002a110.908948 110.908948 0 0 1-31.384994 80.212472L728.625315 456.717656a67.704376 67.704376 0 0 0-20.885079 47.852076c-0.401636 18.245756 6.368801 35.516109 19.049028 48.655348a68.278142 68.278142 0 0 0 96.507425 1.836051l113.376141-109.417157a248.899647 248.899647 0 0 0 72.064997-180.334622c-1.147532-67.933882-28.401412-131.73665-72.983022-175.342859z" />
            <path d="M520.864683 400.259091a68.565025 68.565025 0 0 0-1.893427 97.023814c22.950636 22.778506 35.975122 53.245475 36.606264 85.720625a111.88435 111.88435 0 0 1-32.30302 81.302628l-174.424833 175.055976a121.179358 121.179358 0 0 1-170.752732-171.728134l113.605648-119.974449a68.335519 68.335519 0 0 0-2.52457-96.450048 67.589623 67.589623 0 0 0-48.770101-18.647392c-18.188379 0.459013-35.114473 8.032723-47.737323 21.229339l-112.458116 118.769541c-96.679554 100.638539-94.269737 261.924133 5.106516 359.292206a260.661848 260.661848 0 0 0 183.605088 76.827253h1.204908a244.998039 244.998039 0 0 0 174.31008-72.294503l175.342859-175.629741a249.18653 249.18653 0 0 0 72.122374-180.334623 259.227433 259.227433 0 0 0-77.22889-180.449375 68.507648 68.507648 0 0 0-93.810725 0.286883z" />
          </svg>
          <span class="action-label">{{ busy === `connect:${selectedName}` ? t('common.connecting') : t('common.connect') }}</span>
        </button>
      </div>
    </div>

    <div v-if="overview?.routeWarnings?.length" class="banner warn">
      {{ joinWarnings(overview.routeWarnings) }}
    </div>

    <div class="grid">
      <div class="card metric">
        <div class="metric-heading">
          <span class="label">{{ t('overview.download') }}</span>
          <span class="hint">{{ t('overview.cumulative', { size: formatBytes(snapshot?.rxBytes ?? 0), packets: (snapshot?.rxPackets ?? 0).toLocaleString() }) }}</span>
        </div>
        <strong class="value">{{ formatRate(currentRate.rx) }}</strong>
        <Sparkline :values="rates.map((item) => item.rx)" tone="accent" />
      </div>
      <div class="card metric">
        <div class="metric-heading">
          <span class="label">{{ t('overview.upload') }}</span>
          <span class="hint">{{ t('overview.cumulative', { size: formatBytes(snapshot?.txBytes ?? 0), packets: (snapshot?.txPackets ?? 0).toLocaleString() }) }}</span>
        </div>
        <strong class="value">{{ formatRate(currentRate.tx) }}</strong>
        <Sparkline :values="rates.map((item) => item.tx)" tone="ok" />
      </div>
      <div class="card metric">
        <span class="label">{{ t('overview.routes') }}</span>
        <strong class="value">{{ t('overview.routeCount', { n: overview?.routes.length ?? 0 }) }}</strong>
        <span class="hint">{{ overview?.iface ? t('overview.viaIface', { iface: overview.iface }) : t('overview.noTunnel') }}</span>
        <ul class="route-list mono">
          <li v-for="route in (overview?.routes ?? []).slice(0, 4)" :key="route">{{ route }}</li>
          <li v-if="(overview?.routes?.length ?? 0) > 4" class="hint">…</li>
        </ul>
      </div>
      <div v-if="snapshot" class="connection-panel card">
        <span class="connection-panel-title">{{ t('overview.attributes') }}</span>
        <div class="meta">
          <span v-if="overview?.iface">
            {{ t('overview.interface') }} <span class="mono">{{ overview.iface }} · {{ overview.ifaceIpv4 ?? t('common.noIpv4') }} · MTU {{ overview.ifaceMtu ?? '—' }}</span>
          </span>
        </div>
        <div class="flags">
          <span class="badge" :class="isYes(snapshot.udpAccel) ? 'warn' : 'off'">
            {{ t('overview.udpAccel') }} {{ isYes(snapshot.udpAccel) ? t('overview.active') : t('overview.inactive') }}
          </span>
          <span class="badge off">{{ t('overview.tcpConnections', { n: snapshot.tcpConnections || '—' }) }}</span>
          <span class="badge off">{{ localizeFlag(snapshot.encrypted) || t('overview.encryptionUnknown') }}</span>
        </div>
      </div>
    </div>
  </section>
</template>

<style scoped>
.overview {
  padding: 18px 20px 24px;
  display: flex;
  flex-direction: column;
  gap: 12px;
  flex: 1 1 auto;
  /* 作为 flex 子项允许收缩，超出窗口的内容由本页滚动查看 */
  min-height: 0;
  overflow-y: auto;
}

.spin {
  animation: spin 0.9s linear infinite;
}

@keyframes spin {
  to {
    transform: rotate(360deg);
  }
}

.head {
  display: flex;
  justify-content: space-between;
  align-items: flex-start;
  gap: 16px;
  flex: none;
  top: 0;
  z-index: 2;
  background: var(--bg);
}

.head button {
  display: inline-flex;
  align-items: center;
  gap: 6px;
}

/* 空账号提示：文案 + 动作按钮，竖排左对齐，不要把状态卡撑高太多 */
.no-accounts {
  display: flex;
  flex-direction: column;
  align-items: flex-start;
  gap: 6px;
}

.no-accounts button {
  display: inline-flex;
  align-items: center;
  gap: 6px;
}

.banner {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 12px;
  padding: 8px 12px;
  border-radius: var(--radius-ctl);
  font-size: 12px;
  border: 0.5px solid;
}

/* 进行中的提示：用中性色，别让它看着像出错 */
.banner.busy {
  background: var(--ghost-hover-bg);
  border-color: var(--line-strong);
  color: var(--text);
}

.banner.busy .hint {
  color: var(--text-soft);
}

.banner.warn {
  background: var(--warn-soft);
  border-color: var(--warn-line);
  color: var(--warn);
}

.banner.bad {
  background: var(--danger-soft);
  border-color: var(--danger-line);
  color: var(--danger);
  display: block;
}

.banner.off {
  background: var(--off-bg);
  border-color: var(--off-line);
  color: var(--text-soft);
}

/* 状态卡是连接页的主视觉，但只占一行高度：
   左侧圆形操作按钮，右侧连接信息，指标卡不用滚动就能露出来 */
.status.card {
  display: flex;
  flex: 0 0 auto;
  flex-direction: row;
  align-items: center;
  gap: 22px;
  padding: 18px 24px;
  border: 0;
  background: linear-gradient(180deg, var(--panel) 0%, var(--panel-muted) 100%);
}

.status.connected {
  background: linear-gradient(180deg, var(--panel) 0%, var(--accent-soft) 170%);
}

.status-main {
  display: flex;
  flex-direction: column;
  align-items: flex-start;
  gap: 6px;
  order: 2;
  flex: 1 1 auto;
  min-width: 0;
}

.status-heading {
  display: flex;
  align-items: center;
  flex-wrap: wrap;
  gap: 10px;
  min-width: 0;
}

.status-heading .strong {
  font-size: 20px;
  color: var(--text);
  overflow-wrap: anywhere;
}

.connection-action {
  width: 112px;
  height: 112px;
  border: 0;
  border-radius: 50%;
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  gap: 6px;
  color: white;
  background: linear-gradient(145deg, #4c5bdd, #5967df);
  box-shadow: 0 10px 28px color-mix(in srgb, var(--accent) 24%, transparent);
  transition: transform 160ms ease, box-shadow 160ms ease, opacity 160ms ease;
  order: 1;
}

.disconnect-icon {
  width: 34px;
  height: 34px;
}

.connection-action:hover:not(:disabled) {
  transform: translateY(-2px);
  box-shadow: 0 14px 32px color-mix(in srgb, var(--accent) 32%, transparent);
}

.connection-action:disabled {
  /* 只调透明度会让人以为还点得动：按钮仍是亮蓝渐变。
     真正置灰要换掉背景、去掉投影，并显式给默认光标。 */
  background: linear-gradient(145deg, var(--line-strong), var(--line));
  color: var(--text-soft);
  box-shadow: none;
  cursor: default;
}

.action-label {
  font-size: 12px;
  font-weight: 500;
}

.action-duration {
  font-size: 12px;
  font-weight: 500;
  font-variant-numeric: tabular-nums;
}

.disconnect-action.showing-duration .action-label {
  display: none;
}

.disconnect-action:not(.showing-duration) .action-duration {
  display: none;
}

.disconnect-action.showing-duration:hover .action-duration,
.disconnect-action.showing-duration:focus-visible .action-duration {
  display: none;
}

.disconnect-action.showing-duration:hover .action-label,
.disconnect-action.showing-duration:focus-visible .action-label {
  display: inline;
}

.connect-icon {
  display: block;
}

.strong {
  font-weight: 500;
}

.meta {
  display: flex;
  flex-wrap: wrap;
  gap: 4px 16px;
  color: var(--text-soft);
  font-size: 12px;
}

.connection-panel {
  display: flex;
  flex-direction: column;
  gap: 12px;
  padding: 12px 14px;
}

.connection-panel-title {
  color: var(--text-soft);
  font-size: 12px;
  font-weight: 400;
}

.flags {
  display: flex;
  flex-wrap: wrap;
  gap: 6px;
}

.status-actions {
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  order: 1;
  gap: 12px;
  flex: none;
}

.status-actions button {
  cursor: pointer;
}

/* 未连接时的配置选择器：只占状态行的一角，不把整张卡片撑成表单 */
.picker {
  width: 240px;
  flex: none;
}

.grid {
  display: grid;
  grid-template-columns: repeat(2, minmax(0, 1fr));
  gap: 12px;
  /* 指标行不参与纵向收缩：否则窗口不够高时卡片被压扁而不出现滚动 */
  flex: 0 0 auto;
}

.metric {
  min-width: 0;
  padding: 12px 14px;
  display: flex;
  flex-direction: column;
  gap: 2px;
}

.metric .label {
  color: var(--text-soft);
  font-size: 12px;
}

.metric-heading {
  display: flex;
  align-items: baseline;
  justify-content: space-between;
  gap: 8px;
}

.metric-heading .hint {
  text-align: right;
  white-space: nowrap;
  flex: none;
}

.metric-heading .label {
  white-space: nowrap;
}

.metric .value {
  font-size: 20px;
  font-weight: 500;
  letter-spacing: -0.2px;
}

.route-list {
  list-style: none;
  margin: 4px 0 0;
  padding: 0;
  color: var(--text-soft);
  font-size: 11px;
}

@media (max-width: 480px) {
  .grid {
    grid-template-columns: minmax(0, 1fr);
  }
}
</style>
