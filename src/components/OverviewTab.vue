<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref, watch } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import { RefreshCw, Plug, PlugZap, Power, Pencil, Plus, Trash2, FolderOpen } from 'lucide-vue-next'
import AccountDialog from './AccountDialog.vue'
import Sparkline from './Sparkline.vue'
import {
  busy,
  connectAccount,
  disconnectAccount,
  formatBytes,
  formatDuration,
  formatRate,
  isAccountConnected,
  isYes,
  localizeFlag,
  localizeStatus,
  overview,
  refresh,
  run,
  settings,
  showToast,
  startService,
  stopService,
  toast,
} from '../lib/api'
import { t, locale } from '../lib/i18n'

interface RateSample {
  rx: number
  tx: number
}

const rates = ref<RateSample[]>([])
const errorText = ref('')
const now = ref(Date.now())
const editing = ref<string | null>(null)
const creating = ref(false)
const deleting = ref<string | null>(null)
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

const selected = computed(() => {
  if (!snapshot.value) return null
  return accounts.value.find((account) => account.name === snapshot.value?.name) ?? null
})

/** 账户是否处于已连接状态：中英文输出都认，且 status 可能缺失 */
function isConnected(account: { status?: string | null }): boolean {
  return isAccountConnected(account.status)
}

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

async function openEdit(name: string) {
  editing.value = name
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

async function confirmDelete(name: string) {
  deleting.value = name
}

async function doDelete(name: string) {
  const ok = await run(`delete:${name}`, () => invoke('delete_account', { name }))
  deleting.value = null
  if (ok !== null) showToast(t('overview.deleted', { name }))
}

async function revealConfig() {
  showToast(t('overview.configDir', { path: await invoke<string>('config_dir_path') }))
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
        <p class="hint">
          {{ t('overview.subtitle') }}
        </p>
      </div>
      <div class="row">
        <button class="ghost" :disabled="refreshing" @click="manualRefresh">
          <RefreshCw :size="14" :class="{ spin: refreshing }" /> {{ t('overview.refresh') }}
        </button>
        <button class="primary" :disabled="!!busy" @click="creating = true">
          <Plus :size="14" /> {{ t('overview.newAccount') }}
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
          <span class="badge" :class="snapshot ? 'ok' : 'off'">
            <i class="dot" />{{ snapshot ? t('status.connected') : overview?.serviceRunning ? t('status.disconnected') : t('status.serviceDown') }}
          </span>
          <span v-if="snapshot" class="strong">{{ snapshot.name }}</span>
          <span v-else class="muted">{{ t('overview.chooseAccount') }}</span>
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
          class="danger"
          :disabled="!!busy"
          @click="disconnectAccount(snapshot.name)"
        >
          <Plug :size="14" /> {{ busy === `disconnect:${snapshot.name}` ? t('common.disconnecting') : t('common.disconnect') }}
        </button>
        <button
          v-else-if="accounts.length"
          class="primary"
          :disabled="!!busy || !overview?.serviceRunning"
          @click="connectAccount(settings?.activeAccount || accounts[0].name)"
        >
          <PlugZap :size="14" /> {{ t('common.connect') }} {{ settings?.activeAccount || accounts[0].name }}
        </button>
        <button
          v-if="!snapshot && overview?.serviceRunning"
          class="ghost"
          :disabled="!!busy"
          :title="t('overview.stopTooltip')"
          @click="stopService()"
        >
          <Power :size="14" /> {{ t('overview.stopService') }}
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

    <div class="card list">
      <header class="list-head">
        <h2>{{ t('overview.accountsTitle') }}</h2>
        <span class="hint">{{ t('overview.accountsHint', { n: accounts.length }) }}</span>
      </header>
      <div v-if="!accounts.length" class="empty">
        <p class="muted">{{ t('overview.noAccounts') }}</p>
        <button class="primary" @click="creating = true"><Plus :size="14" /> {{ t('overview.createFirst') }}</button>
      </div>
      <ul>
        <li v-for="account in accounts" :key="account.name" :class="{ active: snapshot?.name === account.name }">
          <div class="acct">
            <div class="row">
              <span class="badge" :class="isConnected(account) ? 'ok' : 'off'">
                <i class="dot" />{{ localizeStatus(account.status) }}
              </span>
              <span class="strong">{{ account.name }}</span>
              <span v-if="settings?.activeAccount === account.name" class="badge off">{{ t('overview.default') }}</span>
            </div>
            <div class="meta">
              <span class="mono">{{ account.host }}:{{ account.port ?? 443 }}</span>
              <span>{{ t('overview.hub') }} <span class="mono">{{ account.hub || '—' }}</span></span>
              <span>{{ t('overview.interface') }} <span class="mono">{{ account.nic || '—' }}</span></span>
            </div>
          </div>
          <div class="row actions">
            <button
              v-if="!isConnected(account)"
              class="ghost"
              :disabled="!!busy || !overview?.serviceRunning"
              @click="connectAccount(account.name)"
            >
              <PlugZap :size="14" /> {{ busy === `connect:${account.name}` ? t('common.connecting') : t('common.connect') }}
            </button>
            <button v-else class="ghost" :disabled="!!busy" @click="disconnectAccount(account.name)">
              <Plug :size="14" /> {{ t('common.disconnect') }}
            </button>
            <button class="ghost" :disabled="!!busy" :title="t('common.edit')" @click="openEdit(account.name)">
              <Pencil :size="14" />
            </button>
            <button class="danger" :disabled="!!busy" :title="t('common.delete')" @click="confirmDelete(account.name)">
              <Trash2 :size="14" />
            </button>
          </div>
        </li>
      </ul>
      <footer class="list-foot">
        <span class="hint">{{ t('overview.clientDir') }}</span>
        <span class="mono">{{ overview?.vpnDir ?? t('common.notFound') }}</span>
        <button class="ghost small" @click="revealConfig"><FolderOpen :size="13" /> {{ t('overview.openConfigDir') }}</button>
      </footer>
    </div>

    <AccountDialog v-if="creating" :name="null" @close="creating = false" />
    <AccountDialog v-if="editing" :name="editing" @close="editing = null" />

    <div v-if="deleting" class="overlay" @click.self="deleting = null">
      <div class="dialog" style="max-width: 420px">
        <header><h2>{{ t('overview.deleteTitle') }}</h2></header>
        <p class="muted">
          {{ t('overview.deletePrefix') }}<span class="mono">{{ deleting }}</span>{{ t('overview.deleteSuffix') }}
        </p>
        <div class="row" style="justify-content: flex-end; margin-top: 16px">
          <button class="ghost" @click="deleting = null">{{ t('common.cancel') }}</button>
          <button class="danger" @click="doDelete(deleting)">{{ t('common.delete') }}</button>
        </div>
      </div>
    </div>

    <div v-if="toast" class="toast" :class="{ error: toast.error }">{{ toast.text }}</div>
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
  overflow: auto;
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
  /* 滚动时标题始终可见：页头流失会让人以为界面在跳 */
  position: sticky;
  top: 0;
  z-index: 2;
  padding-bottom: 10px;
  margin-top: -18px;
  padding-top: 18px;
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

.status {
  display: flex;
  justify-content: space-between;
  gap: 16px;
  padding: 14px 16px;
  border-left: 3px solid var(--line-strong);
}

.status.connected {
  border-left-color: var(--ok);
}

.status-main {
  display: flex;
  flex-direction: column;
  gap: 6px;
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
  align-items: center;
  gap: 8px;
  flex: none;
}

.status-actions button {
  display: inline-flex;
  gap: 6px;
  align-items: center;
}

.grid {
  display: grid;
  grid-template-columns: repeat(3, minmax(0, 1fr));
  gap: 12px;
  /* 指标行不参与纵向收缩：否则窗口不够高时卡片被压扁而不出现滚动 */
  flex: none;
}

.metric {
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

.list {
  padding: 0;
  overflow: hidden;
}

.list-head {
  display: flex;
  justify-content: space-between;
  align-items: baseline;
  padding: 12px 16px;
  border-bottom: 0.5px solid var(--line);
}

.list ul {
  list-style: none;
  margin: 0;
  padding: 0;
}

.list li {
  display: flex;
  justify-content: space-between;
  align-items: center;
  gap: 12px;
  padding: 10px 16px;
  border-bottom: 0.5px solid var(--line);
}

.list li:last-child {
  border-bottom: none;
}

.list li.active {
  background: var(--ok-soft);
}

.acct {
  display: flex;
  flex-direction: column;
  gap: 3px;
  min-width: 0;
}

.actions {
  flex-shrink: 0;
}

.actions button {
  display: inline-flex;
  align-items: center;
  gap: 5px;
  min-height: 30px;
  padding: 4px 10px;
}

.empty {
  padding: 28px 16px;
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: 10px;
}

.empty button {
  display: inline-flex;
  gap: 6px;
  align-items: center;
}

.list-foot {
  display: flex;
  gap: 8px;
  align-items: center;
  padding: 8px 16px;
  background: var(--panel-muted);
  border-top: 0.5px solid var(--line);
}

.list-foot button.small {
  margin-left: auto;
  min-height: 26px;
  padding: 2px 8px;
  display: inline-flex;
  gap: 5px;
  align-items: center;
  font-size: 12px;
}
</style>
