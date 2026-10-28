<script setup lang="ts">
import { computed, onMounted, ref, watch } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import { Pencil, PlugZap, Plus, Power, RefreshCw, Trash2 } from 'lucide-vue-next'
import { Tooltip as ATooltip } from '@arco-design/web-vue'
import '@arco-design/web-vue/es/tooltip/style/css.js'
import DisconnectIcon from './DisconnectIcon.vue'
import InfoTip from './InfoTip.vue'
import { confirmDialog, openAccountDialog } from '../lib/dialog'
import {
  busy,
  clientAccountGroups,
  connectAccount,
  connectInClient,
  disconnectAccount,
  isAccountConnected,
  isAccountConnecting,
  localizeStatus,
  overview,
  refresh,
  run,
  settings,
  showToast,
  startService,
  type ClientAccountGroup,
  type ClientSource,
} from '../lib/api'
import { t } from '../lib/i18n'

const accounts = computed(() => overview.value?.accounts ?? [])

/** 表格里的一行，既可能是实时账号（当前客户端），也可能是静态读出来的 */
interface RowAccount {
  name: string
  host: string
  port?: number | null
  hub: string
  nic: string
  /** 实时状态：只有当前客户端的账号有 */
  status?: string | null
  /** 配置文件里有没有设密码：只有静态读出来的有 */
  hasPassword?: boolean
}

interface AccountGroup {
  dir: string
  label: string
  current: boolean
  /** 这份客户端的来源；跨客户端连接时要原样回传，写错会把内置客户端记成「自己指定」 */
  source: ClientSource
  accounts: RowAccount[]
}

/** 各客户端目录的静态清单（服务没跑时也能读到） */
const rawGroups = ref<ClientAccountGroup[]>([])

async function loadGroups() {
  try {
    rawGroups.value = await clientAccountGroups()
  } catch {
    rawGroups.value = []
  }
}

const refreshing = ref(false)

/**
 * 手动刷新：丢开缓存重读一次状态，并重新静态读一遍各客户端的配置清单。
 *
 * 这一页没有轮询（2 秒轮询只在「连接」页跑），在外面改了配置文件再回来，
 * 列表就是旧的。转圈留最短时长：缓存没命中时重读只要几毫秒，图标闪一下
 * 等于没反馈。
 */
const MIN_SPIN_MS = 450

async function manualRefresh() {
  if (refreshing.value) return
  refreshing.value = true
  const startedAt = Date.now()
  try {
    await refresh(true)
    await loadGroups()
    const elapsed = Date.now() - startedAt
    if (elapsed < MIN_SPIN_MS) {
      await new Promise((resolve) => window.setTimeout(resolve, MIN_SPIN_MS - elapsed))
    }
    showToast(t('overview.refreshed'))
  } catch (error) {
    showToast(String(error), true)
  } finally {
    refreshing.value = false
  }
}

/**
 * 列表数据源。
 *
 * 当前客户端用**实时账号**（带连接状态、可编辑删除）；其它客户端用配置文件里
 * 静态读出来的（只读展示，点连接会先切过去）。
 */
const groups = computed<AccountGroup[]>(() => {
  const live = accounts.value
  const list: AccountGroup[] = rawGroups.value.map((group) => {
    if (!group.current) return group
    // 服务在跑且读到了账号 → 用实时的；否则退回静态，至少能看见有哪些配置
    const rows: RowAccount[] = live.length ? live.map((account) => ({ ...account })) : group.accounts
    return { ...group, accounts: rows }
  })

  // 当前客户端一定要有一组。
  //
  // 后端那份静态清单可能**不含**当前客户端：配置文件读不到（root 所有 0600）
  // 或刚建好还没落盘时，它会被当成"没有配置"直接过滤掉。这时候如果只渲染
  // 后端给的组，用户刚新建的配置就完全不显示——看着像"保存了没刷新"。
  // 所以这里用实时数据补一组兜底。
  if (!list.some((group) => group.current)) {
    list.unshift({
      dir: overview.value?.vpnDir ?? '',
      label: t('profiles.currentClient'),
      current: true,
      source: settings.value?.clientSource ?? 'bundled',
      accounts: live.map((account) => ({ ...account })),
    })
  }
  return list
})

/**
 * 当前客户端的实时状态。
 *
 * 三态而不是两态：客户端连不上服务器时会无限重连，AccountList 一直报「连接中」。
 * 把这种状态画成绿色「已连接」，用户就会遇到"报了连接超时、界面却说已连接"
 * 这种自相矛盾的画面（真踩过）。「连接中」必须有自己的一档。
 */
function liveStatus(name: string): 'connected' | 'connecting' | 'disconnected' | null {
  const account = accounts.value.find((item) => item.name === name)
  if (!account) return null
  if (isConnected(account)) return 'connected'
  return isAccountConnecting(account.status) ? 'connecting' : 'disconnected'
}

/** 状态徽标的配色：已连接绿、连接中黄、其它灰 */
function badgeTone(name: string): string {
  const state = liveStatus(name)
  return state === 'connected' ? 'ok' : state === 'connecting' ? 'warn' : 'off'
}

/** 状态徽标的文字：连接中要如实说，别拿原始的"重试"去糊弄人 */
function badgeText(name: string): string {
  const state = liveStatus(name)
  if (state === 'connected') return t('status.connected')
  if (state === 'connecting') return t('status.connecting')
  return localizeStatus(accountOf(name)?.status)
}

function accountOf(name: string) {
  return accounts.value.find((item) => item.name === name) ?? null
}

/** 待确认的跨客户端连接 */
/**
 * 点「切换并连接」。
 *
 * 同一个客户端里没跑服务时**直接连**：do_connect 自己会把服务拉起来，
 * 而走 connect_in_client 会把来源重写一遍——内置客户端会被记成
 * 「自己指定」，下次打开客户端页就错位了。
 * 别的客户端则必须问一句——切过去会停掉当前服务、断掉正在用的隧道。
 */
async function askSwitchConnect(group: AccountGroup, name: string) {
  if (group.current) {
    void connectAccount(name)
    return
  }
  const ok = await confirmDialog({
    title: t('profiles.switchConnectTitle'),
    body: t('profiles.switchConnectBody', { name, dir: group.dir }),
    confirmText: t('profiles.switchAndConnect'),
    danger: true,
  })
  // 来源按这份客户端的真实来源回传：一律传 custom 会把内置那份记成自己指定
  if (ok) void connectInClient(group.dir, name, group.source)
}

/** 账户是否处于已连接状态：中英文输出都认，且 status 可能缺失 */
function isConnected(account: { status?: string | null }): boolean {
  return isAccountConnected(account.status)
}

function openEdit(name: string) {
  void openAccountDialog(name)
}

async function confirmDelete(name: string) {
  const ok = await confirmDialog({
    title: t('profiles.deleteTitle'),
    body: `${t('profiles.deletePrefix')}${name}${t('profiles.deleteSuffix')}`,
    confirmText: t('common.delete'),
    danger: true,
  })
  if (!ok) return
  const result = await run(`delete:${name}`, () => invoke('delete_account', { name }))
  if (result !== null) showToast(t('profiles.deleted', { name }))
}

/** 打开该组自己的客户端目录（不是应用设置目录：配置跟着目录走，各组各的） */
async function openGroupDir(group: AccountGroup) {
  try {
    // 打开后文件管理器自己会把目录亮出来，那就是最好的反馈，
    // 再弹一条"已打开 /home/xxx"纯属噪音。只有真的打不开才提示。
    await invoke<string>('open_dir', { path: group.dir })
  } catch (error) {
    showToast(String(error), true)
  }
}

// 连接页负责轮询；这里进页面时补一次，避免停在别的页签时状态变旧
watch(
  () => [overview.value?.serviceRunning, accounts.value.length] as const,
  () => void loadGroups(),
  { immediate: true },
)

onMounted(() => {
  void refresh().catch(() => {})
  void loadGroups()
})
</script>

<template>
  <section class="profiles">
    <header class="head">
      <div>
        <div class="title-line">
          <h1>{{ t('profiles.title') }}</h1>
          <InfoTip :text="t('profiles.subtitle')" />
          <span class="hint">{{ t('profiles.accountsHint', { n: accounts.length }) }}</span>
        </div>
      </div>
      <div class="row">
        <button
          class="ghost"
          :disabled="refreshing"
          :aria-label="t('overview.refresh')"
          :title="t('profiles.refreshTooltip')"
          @click="manualRefresh"
        >
          <RefreshCw :size="16" :class="{ spin: refreshing }" />
        </button>
        <button class="primary" :disabled="!!busy" @click="openAccountDialog(null)">
          <Plus :size="14" /> {{ t('profiles.newAccount') }}
        </button>
      </div>
    </header>

    <div v-if="!overview?.vpncmdReady" class="banner warn">
      {{ t('overview.vpncmdMissingPrefix') }}<span class="mono">vpncmd</span>{{ t('overview.vpncmdMissingSuffix') }}
    </div>

    <div v-if="overview && !overview.serviceRunning" class="banner off">
      <span>{{ t('profiles.serviceDown') }}</span>
      <button class="ghost" :disabled="!!busy" @click="startService">
        <Power :size="14" /> {{ t('overview.startService') }}
      </button>
    </div>

    <!-- 每个客户端目录一组。当前那份可编辑/删除/看状态；
         其它那份只读展示 + 「切换并连接」——配置是跟着客户端目录走的，
         只列当前那份等于把别处的配置藏起来，用户会以为丢了。 -->
    <div v-for="group in groups" :key="group.dir" class="card list" :class="{ other: !group.current }">
      <header class="group-head">
        <span class="strong">{{ group.label }}</span>
        <!-- 路径即「打开目录」入口：悬浮看全文（长路径会被省略号截断），点击打开 -->
        <ATooltip v-if="group.dir" position="bottom">
          <template #content>
            <div class="mono">{{ group.dir }}</div>
            <div class="dir-tip-hint">{{ t('profiles.openDir') }}</div>
          </template>
          <span
            class="mono hint group-dir"
            role="button"
            tabindex="0"
            :aria-label="t('profiles.openDir')"
            @click="openGroupDir(group)"
            @keydown.enter.prevent="openGroupDir(group)"
          >
            {{ group.dir }}
          </span>
        </ATooltip>
        <span v-else class="mono hint group-dir">{{ t('common.notFound') }}</span>
        <span class="badge" :class="group.current ? 'ok' : 'off'">
          {{ group.current ? t('profiles.currentClient') : t('profiles.otherClient') }}
        </span>
      </header>

      <div v-if="!group.accounts.length" class="empty">
        <p class="muted">{{ t('profiles.noAccounts') }}</p>
        <button v-if="group.current" class="primary" @click="openAccountDialog(null)">
          <Plus :size="14" /> {{ t('profiles.createFirst') }}
        </button>
      </div>

      <ul v-else>
        <li
          v-for="account in group.accounts"
          :key="account.name"
          :class="{ active: group.current && overview?.snapshot?.name === account.name }"
        >
          <div class="acct">
            <div class="row">
              <!-- 状态徽标只有"当前客户端 + 服务在跑"时才有意义 -->
              <span
                v-if="group.current && overview?.serviceRunning && liveStatus(account.name)"
                class="badge"
                :class="badgeTone(account.name)"
              >
                <i class="dot" />{{ badgeText(account.name) }}
              </span>
              <span class="strong">{{ account.name }}</span>
              <span v-if="group.current && settings?.activeAccount === account.name" class="badge off">
                {{ t('profiles.default') }}
              </span>
              <span v-if="'hasPassword' in account && !account.hasPassword" class="badge warn">
                {{ t('profiles.noPassword') }}
              </span>
            </div>
            <div class="meta">
              <span class="mono">{{ account.host }}:{{ account.port ?? 443 }}</span>
              <span>{{ t('overview.hub') }} <span class="mono">{{ account.hub || '—' }}</span></span>
              <span>{{ t('overview.interface') }} <span class="mono">{{ account.nic || '—' }}</span></span>
            </div>
          </div>
          <div class="row actions">
            <!-- 当前客户端：走原来的就近连接/断开。
                 连接中也是「断开」：客户端不许对同一配置重复 AccountConnect
                 （错误代码 35），点了"连接"只会收到一句报错；想重试就先断开。 -->
            <template v-if="group.current && overview?.serviceRunning">
              <button
                v-if="liveStatus(account.name) === 'disconnected'"
                class="ghost"
                :disabled="!!busy"
                @click="connectAccount(account.name)"
              >
                <PlugZap :size="14" />
                {{ busy === `connect:${account.name}` ? t('common.connecting') : t('common.connect') }}
              </button>
              <button v-else class="ghost" :disabled="!!busy" @click="disconnectAccount(account.name)">
                <DisconnectIcon /> {{ t('common.disconnect') }}
              </button>
              <button class="ghost" :disabled="!!busy" :title="t('common.edit')" @click="openEdit(account.name)">
                <Pencil :size="14" />
              </button>
              <button class="danger" :disabled="!!busy" :title="t('common.delete')" @click="confirmDelete(account.name)">
                <Trash2 :size="14" />
              </button>
            </template>
            <!-- 其它客户端（或服务没跑）：一键切过去并连上。
                 当前客户端其实不用"切"，按钮就叫「连接」 -->
            <button v-else class="ghost" :disabled="!!busy" @click="askSwitchConnect(group, account.name)">
              <PlugZap :size="14" />
              {{ group.current ? t('common.connect') : t('profiles.switchAndConnect') }}
            </button>
          </div>
        </li>
      </ul>
    </div>
  </section>
</template>

<style scoped>
.profiles {
  padding: 18px 20px 24px;
  display: flex;
  flex-direction: column;
  gap: 12px;
  height: 100%;
  /* 作为 flex 子项必须允许收缩到内容以下，否则 height:100% 撑破容器、标题被裁 */
  min-height: 0;
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
  background: var(--bg);
}

.head button {
  display: inline-flex;
  align-items: center;
  gap: 6px;
}

.title-line {
  display: flex;
  align-items: baseline;
  flex-wrap: wrap;
  gap: 8px;
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
  flex: none;
}

.banner.warn {
  background: var(--warn-soft);
  border-color: var(--warn-line);
  color: var(--warn);
}

.banner.off {
  background: var(--off-bg);
  border-color: var(--off-line);
  color: var(--text-soft);
}

.list {
  padding: 0;
  overflow: hidden;
  flex: none;
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

/* 分组卡片：当前客户端正常显示，其它客户端整体弱化——
   它们在这里只是"看得见、能一键切过去"，不能就地编辑 */
.group-head {
  display: flex;
  align-items: center;
  gap: 8px;
  flex-wrap: wrap;
  padding: 12px 16px 8px;
  border-bottom: 0.5px solid var(--line);
}

/* 路径尽量铺满标题与徽标之间的空位，放不下才截断。
   flex-basis: 0 让它退出换行判定——nowrap 的长路径按内容宽度参与折行时
   会被整条挤到第二行；max-content 又把点击区收回文字本身。 */
.group-dir {
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  flex: 1 1 0;
  min-width: 0;
  max-width: max-content;
}

/* 路径可点开目录：给个链接样 affordance，不然没人知道能点 */
span.group-dir[role='button'] {
  cursor: pointer;
  text-decoration: underline;
  text-decoration-color: transparent;
  text-underline-offset: 3px;
  transition: text-decoration-color 0.15s, color 0.15s;
}

span.group-dir[role='button']:hover,
span.group-dir[role='button']:focus-visible {
  color: var(--text);
  text-decoration-color: currentColor;
}

/* 悬浮提示里的"打开目录"副行 */
.dir-tip-hint {
  margin-top: 2px;
  font-size: 11px;
  opacity: 0.75;
}

/* 「当前客户端/其它客户端」徽标靠右 */
.group-head .badge {
  margin-left: auto;
}

.list.other {
  background: var(--panel-muted);
  border-style: dashed;
}

.list.other .strong {
  color: var(--text-soft);
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

.spin {
  animation: spin 0.9s linear infinite;
}

@keyframes spin {
  to {
    transform: rotate(360deg);
  }
}
</style>
