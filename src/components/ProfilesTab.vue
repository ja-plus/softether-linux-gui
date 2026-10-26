<script setup lang="ts">
import { computed, onMounted, ref } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import { FolderOpen, Pencil, PlugZap, Plus, Power, Trash2 } from 'lucide-vue-next'
import AccountDialog from './AccountDialog.vue'
import DisconnectIcon from './DisconnectIcon.vue'
import InfoTip from './InfoTip.vue'
import {
  busy,
  connectAccount,
  disconnectAccount,
  isAccountConnected,
  localizeStatus,
  overview,
  refresh,
  run,
  settings,
  showToast,
  startService,
} from '../lib/api'
import { t } from '../lib/i18n'

const editing = ref<string | null>(null)
const creating = ref(false)
const deleting = ref<string | null>(null)

const accounts = computed(() => overview.value?.accounts ?? [])

/** 账户是否处于已连接状态：中英文输出都认，且 status 可能缺失 */
function isConnected(account: { status?: string | null }): boolean {
  return isAccountConnected(account.status)
}

function openEdit(name: string) {
  editing.value = name
}

function confirmDelete(name: string) {
  deleting.value = name
}

async function doDelete(name: string) {
  const ok = await run(`delete:${name}`, () => invoke('delete_account', { name }))
  deleting.value = null
  if (ok !== null) showToast(t('profiles.deleted', { name }))
}

async function revealConfig() {
  try {
    // 后端负责拉起 xdg-open，路径回传只为给用户一个确认提示
    showToast(t('profiles.configDir', { path: await invoke<string>('open_config_dir') }))
  } catch (error) {
    showToast(String(error), true)
  }
}

// 连接页负责轮询；这里进页面时补一次，避免停在别的页签时状态变旧
onMounted(() => {
  void refresh().catch(() => {})
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
        <button class="primary" :disabled="!!busy" @click="creating = true">
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

    <div class="card list">
      <div v-if="!accounts.length" class="empty">
        <p class="muted">{{ t('profiles.noAccounts') }}</p>
        <button class="primary" @click="creating = true"><Plus :size="14" /> {{ t('profiles.createFirst') }}</button>
      </div>
      <ul>
        <li v-for="account in accounts" :key="account.name" :class="{ active: overview?.snapshot?.name === account.name }">
          <div class="acct">
            <div class="row">
              <span class="badge" :class="isConnected(account) ? 'ok' : 'off'">
                <i class="dot" />{{ localizeStatus(account.status) }}
              </span>
              <span class="strong">{{ account.name }}</span>
              <span v-if="settings?.activeAccount === account.name" class="badge off">{{ t('profiles.default') }}</span>
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
              <DisconnectIcon /> {{ t('common.disconnect') }}
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
        <span class="hint">{{ t('profiles.clientDir') }}</span>
        <span class="mono">{{ overview?.vpnDir ?? t('common.notFound') }}</span>
        <button class="ghost small" @click="revealConfig"><FolderOpen :size="13" /> {{ t('profiles.openConfigDir') }}</button>
      </footer>
    </div>

    <AccountDialog v-if="creating" :name="null" @close="creating = false" />
    <AccountDialog v-if="editing" :name="editing" @close="editing = null" />

    <div v-if="deleting" class="overlay" @click.self="deleting = null">
      <div class="dialog" style="max-width: 420px">
        <header><h2>{{ t('profiles.deleteTitle') }}</h2></header>
        <p class="muted">
          {{ t('profiles.deletePrefix') }}<span class="mono">{{ deleting }}</span>{{ t('profiles.deleteSuffix') }}
        </p>
        <div class="row" style="justify-content: flex-end; margin-top: 16px">
          <button class="ghost" @click="deleting = null">{{ t('common.cancel') }}</button>
          <button class="danger" @click="doDelete(deleting)">{{ t('common.delete') }}</button>
        </div>
      </div>
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
