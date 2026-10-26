<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref, watch } from 'vue'
import { RefreshCw, Plug, PlugZap, Power } from 'lucide-vue-next'
import SelectInput, { type SelectOption } from './SelectInput.vue'
import Sparkline from './Sparkline.vue'
import {
  busy,
  connectAccount,
  disconnectAccount,
  formatBytes,
  formatDuration,
  formatRate,
  isYes,
  localizeFlag,
  overview,
  refresh,
  settings,
  startService,
  stopService,
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
const accounts = computed(() => overview.value?.accounts ?? [])
const currentRate = computed(() => rates.value[rates.value.length - 1] ?? { rx: 0, tx: 0 })
const elapsed = computed(() => {
  const since = snapshot.value?.connectedSince
  if (!since) return null
  return Math.max(0, now.value / 1000 - since)
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

/** 手动刷新：只锁自己这一个按钮，不牵动其他按钮 */
async function manualRefresh() {
  if (refreshing.value) return
  refreshing.value = true
  try {
    await tick()
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
        <button
          v-if="!snapshot && overview?.serviceRunning"
          class="ghost"
          :disabled="!!busy"
          :title="t('overview.stopTooltip')"
          @click="stopService()"
        >
          <Power :size="14" /> {{ t('overview.stopService') }}
        </button>
        <button
          class="ghost refresh-icon"
          :disabled="refreshing"
          :aria-label="t('overview.refresh')"
          :title="t('overview.refresh')"
          @click="manualRefresh"
        >
          <RefreshCw :size="16" :class="{ spin: refreshing }" />
        </button>
      </div>
    </header>

    <div v-if="errorText" class="banner bad">
      {{ t('overview.readError', { error: errorText }) }}
      <span class="hint">{{ t('overview.readErrorHint') }}</span>
    </div>

    <div v-if="!overview?.vpncmdReady" class="banner warn">
      {{ t('overview.vpncmdMissingPrefix') }}<span class="mono">vpncmd</span>{{ t('overview.vpncmdMissingSuffix') }}
    </div>

    <div v-if="overview && !overview.serviceRunning" class="banner off">
      <span>{{ t('overview.serviceDown') }}</span>
      <button class="ghost" :disabled="!!busy" @click="startService">
        <Power :size="14" /> {{ t('overview.startService') }}
      </button>
    </div>

    <div class="status card" :class="{ connected: !!snapshot }">
      <div class="status-main">
        <div class="row">
          <span class="badge" :class="[snapshot ? 'ok' : 'off', { 'disconnected-status': !snapshot }]">
            <i class="dot" />{{ snapshot ? t('status.connected') : overview?.serviceRunning ? t('status.disconnected') : t('status.serviceDown') }}
          </span>
          <span v-if="snapshot" class="strong">{{ snapshot.name }}</span>
          <div v-else-if="accounts.length" class="picker">
            <SelectInput
              v-model="selectedName"
              :options="accountOptions"
              :disabled="!overview?.serviceRunning"
            />
          </div>
          <span v-else class="muted">{{ t('overview.noAccounts') }}</span>
        </div>
        <div class="meta">
          <span v-if="snapshot">{{ t('overview.server') }} <span class="mono">{{ snapshot.host }}</span></span>
          <span v-if="snapshot">{{ t('overview.hub') }} <span class="mono">{{ snapshot.hub || '—' }}</span></span>
          <span v-if="overview?.iface">
            {{ t('overview.interface') }} <span class="mono">{{ overview.iface }} · {{ overview.ifaceIpv4 ?? t('common.noIpv4') }} · MTU {{ overview.ifaceMtu ?? '—' }}</span>
          </span>
          <span v-if="elapsed !== null">{{ t('overview.connectedFor', { duration: formatDuration(elapsed) }) }}</span>
        </div>
        <div v-if="snapshot" class="flags">
          <span class="badge" :class="isYes(snapshot.udpAccel) ? 'warn' : 'off'">
            {{ t('overview.udpAccel') }} {{ isYes(snapshot.udpAccel) ? t('overview.active') : t('overview.inactive') }}
          </span>
          <span class="badge off">{{ t('overview.tcpConnections', { n: snapshot.tcpConnections || '—' }) }}</span>
          <span class="badge off">{{ localizeFlag(snapshot.encrypted) || t('overview.encryptionUnknown') }}</span>
        </div>
      </div>

      <div class="status-actions">
        <button
          v-if="snapshot"
          class="connection-action disconnect-action"
          :disabled="!!busy"
          @click="disconnectAccount(snapshot.name)"
        >
          <Plug :size="34" />
          <span class="action-label">{{ busy === `disconnect:${snapshot.name}` ? t('common.disconnecting') : t('common.disconnect') }}</span>
        </button>
        <button
          v-else-if="accounts.length"
          class="connection-action connect-action"
          :disabled="!!busy || !overview?.serviceRunning || !selectedName"
          @click="connectAccount(selectedName)"
        >
          <PlugZap :size="34" />
          <span class="action-label">{{ busy === `connect:${selectedName}` ? t('common.connecting') : t('common.connect') }}</span>
        </button>
      </div>
    </div>

    <div v-if="overview?.routeWarnings?.length" class="banner warn">
      {{ joinWarnings(overview.routeWarnings) }}
    </div>

    <div class="grid">
      <div class="card metric">
        <span class="label">{{ t('overview.download') }}</span>
        <strong class="value">{{ formatRate(currentRate.rx) }}</strong>
        <span class="hint">{{ t('overview.cumulative', { size: formatBytes(snapshot?.rxBytes ?? 0), packets: (snapshot?.rxPackets ?? 0).toLocaleString() }) }}</span>
        <Sparkline :values="rates.map((item) => item.rx)" tone="accent" />
      </div>
      <div class="card metric">
        <span class="label">{{ t('overview.upload') }}</span>
        <strong class="value">{{ formatRate(currentRate.tx) }}</strong>
        <span class="hint">{{ t('overview.cumulative', { size: formatBytes(snapshot?.txBytes ?? 0), packets: (snapshot?.txPackets ?? 0).toLocaleString() }) }}</span>
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
    </div>
  </section>
</template>

<style scoped>
.overview {
  padding: 18px 20px 24px;
  display: flex;
  flex-direction: column;
  gap: 12px;
  height: 100%;
  /* 作为 flex 子项必须允许收缩到内容以下，否则 height:100% 撑破容器、标题被裁 */
  min-height: 0;
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

.status.card {
  position: relative;
  display: flex;
  flex: 0 1 260px;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  gap: 18px;
  padding: 30px 22px 24px;
  min-height: 220px;
  max-height: 260px;
  border: 0;
  background: linear-gradient(180deg, var(--panel) 0%, var(--panel-muted) 100%);
  text-align: center;
}

.status.connected {
  background: linear-gradient(180deg, var(--panel) 0%, var(--accent-soft) 170%);
}

.status-main {
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: 10px;
  order: 2;
  width: 100%;
}

.status-main > .row {
  justify-content: center;
  flex-wrap: wrap;
  gap: 8px;
}

.status-main > .row .strong,
.status-main > .row .muted {
  font-size: 20px;
  color: var(--text);
}

.status-main > .row .badge {
  order: 2;
}

.status-main > .row .disconnected-status {
  position: absolute;
  top: 22px;
  left: 22px;
  order: 0;
}

.connection-action {
  width: 120px;
  height: 120px;
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

.connection-action:hover:not(:disabled) {
  transform: translateY(-2px);
  box-shadow: 0 14px 32px color-mix(in srgb, var(--accent) 32%, transparent);
}

.connection-action:disabled {
  opacity: .65;
}

.action-label {
  font-size: 12px;
  font-weight: 500;
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
  display: flex;
  gap: 12px;
  flex-wrap: wrap;
  /* 指标行不参与纵向收缩：否则窗口不够高时卡片被压扁而不出现滚动 */
  flex: 0 0 auto;
}

.metric {
  flex: 1 1 180px;
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
</style>
