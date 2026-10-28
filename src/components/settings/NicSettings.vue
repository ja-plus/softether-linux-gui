<script setup lang="ts">
import { computed, onMounted, ref } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import { Loader2, Plus, Power, Trash2 } from 'lucide-vue-next'
import { confirmDialog } from '../../lib/dialog'
import { overview, refresh, showToast, type NicInfo } from '../../lib/api'
import { t } from '../../lib/i18n'

/**
 * 虚拟网络适配器管理。
 *
 * 为什么值得单独一张卡：适配器是**连接的前提**——没有虚拟网卡，新建的连接配置
 * 根本没有网卡可绑，连上也无处落地。以前只能靠新建配置弹窗里的"创建"按钮，
 * 用户不新建配置就永远看不到它，删/停更没入口。
 */
const nics = computed<NicInfo[]>(() => overview.value?.nics ?? [])

/**
 * 适配器清单是 vpncmd 读出来的，客户端没跑就一定是空的。
 *
 * 这一页以前埋在设置里最底下、没什么人单独进；现在它是左侧一级入口，
 * 空列表如果只说"还没有任何适配器"，会让人以为自己的网卡丢了，
 * 实际上只是没启动客户端。所以这里分两种情况说。
 */
const serviceReady = computed(
  () => !!overview.value?.serviceRunning && !!overview.value?.vpncmdReady,
)

const newName = ref('')
const creating = ref(false)
/** 正在操作的那张网卡（禁用/启用/删除），用于只锁对应行 */
const pending = ref<string | null>(null)

const canCreate = computed(() => !!newName.value.trim() && !creating.value && serviceReady.value)

/** 只替换 nics 字段。overview 为 null 时（首屏还没到）什么都不做，别把整个对象弄坏 */
function applyNics(list: NicInfo[]) {
  if (!overview.value) return
  overview.value = { ...overview.value, nics: list }
}

onMounted(() => {
  // 设置页没有轮询，进页面时主动读一次，别拿启动时的旧清单
  void refresh().catch(() => {})
})

async function create() {
  const name = newName.value.trim()
  if (!name) return
  creating.value = true
  try {
    applyNics(await invoke<NicInfo[]>('create_nic', { name }))
    newName.value = ''
    showToast(t('nic.created', { name }))
  } catch (error) {
    showToast(String(error), true)
  } finally {
    creating.value = false
  }
}

async function toggle(nic: NicInfo) {
  if (pending.value) return
  pending.value = nic.name
  try {
    applyNics(
      await invoke<NicInfo[]>('set_nic_enabled', { name: nic.name, enabled: !nic.enabled }),
    )
    showToast(nic.enabled ? t('nic.disabled', { name: nic.name }) : t('nic.enabled', { name: nic.name }))
  } catch (error) {
    showToast(String(error), true)
  } finally {
    pending.value = null
  }
}

async function askDelete(name: string) {
  const ok = await confirmDialog({
    title: t('nic.deleteTitle'),
    body: t('nic.deleteBody', { name }),
    confirmText: t('common.delete'),
    danger: true,
  })
  if (!ok) return
  pending.value = name
  try {
    applyNics(await invoke<NicInfo[]>('delete_nic', { name }))
    showToast(t('nic.deleted', { name }))
  } catch (error) {
    showToast(String(error), true)
  } finally {
    pending.value = null
  }
}
</script>

<template>
  <div class="card block">
    <div class="row-between">
      <div>
        <h2>{{ t('nic.title') }}</h2>
        <p class="hint" style="margin: 4px 0 0">{{ t('nic.intro') }}</p>
      </div>
    </div>

    <!-- 空状态要说清后果：没有适配器时新建的连接配置绑不上网卡。
         客户端没跑时列表必然为空，这时说的是另一件事（去启动服务）。 -->
    <p v-if="!serviceReady" class="hint empty">{{ t('nic.needService') }}</p>
    <p v-else-if="!nics.length" class="hint empty">{{ t('nic.empty') }}</p>

    <ul v-else class="nic-list">
      <li v-for="nic in nics" :key="nic.name">
        <div class="nic-main">
          <div class="nic-head">
            <span class="strong">{{ nic.name }}</span>
            <span class="badge" :class="nic.enabled ? 'ok' : 'off'">{{ nic.enabled ? t('common.enabled') : t('common.disabled') }}</span>
          </div>
          <span v-if="nic.mac" class="mono hint">{{ nic.mac }}</span>
        </div>
        <div class="nic-actions">
          <button class="ghost small" :disabled="!!pending" @click="toggle(nic)">
            <Loader2 v-if="pending === nic.name" :size="13" class="spin" />
            <Power v-else :size="13" />
            {{ nic.enabled ? t('nic.disable') : t('nic.enable') }}
          </button>
          <button
            class="danger small"
            :disabled="!!pending"
            :title="t('nic.delete')"
            @click="askDelete(nic.name)"
          >
            <Trash2 :size="13" />
            {{ t('nic.delete') }}
          </button>
        </div>
      </li>
    </ul>

    <div class="row nic-create">
      <input
        v-model="newName"
        type="text"
        :placeholder="t('nic.newPlaceholder')"
        @keyup.enter="create"
      />
      <button class="primary" :disabled="!canCreate" :title="serviceReady ? undefined : t('nic.needService')" @click="create">
        <Loader2 v-if="creating" :size="13" class="spin" />
        <Plus v-else :size="13" />
        {{ t('nic.create') }}
      </button>
    </div>
    <p class="hint" style="margin: 6px 0 0">{{ t('nic.nameHint') }}</p>
  </div>
</template>

<style scoped>
.nic-list {
  list-style: none;
  margin: 0;
  padding: 0;
  display: flex;
  flex-direction: column;
  gap: 8px;
}

.nic-list li {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 12px;
  padding: 10px 12px;
  border: 0.5px solid var(--line);
  border-radius: var(--radius-ctl);
}

.nic-main {
  display: flex;
  flex-direction: column;
  gap: 4px;
  min-width: 0;
}

.nic-head {
  display: flex;
  align-items: center;
  gap: 8px;
}

.nic-actions {
  display: flex;
  gap: 8px;
  flex: none;
}

.nic-actions button {
  display: inline-flex;
  align-items: center;
  gap: 6px;
}

.nic-create {
  margin-top: 12px;
  align-items: center;
  gap: 8px;
}

.nic-create input {
  flex: 1;
  max-width: 260px;
}

.nic-create button {
  display: inline-flex;
  align-items: center;
  gap: 6px;
  flex: none;
}

.empty {
  margin: 0;
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
