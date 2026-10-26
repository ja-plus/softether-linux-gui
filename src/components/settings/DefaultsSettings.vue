<script setup lang="ts">
import { computed, onMounted, ref } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import SelectInput, { type SelectOption } from '../SelectInput.vue'
import { overview, showToast } from '../../lib/api'
import { t } from '../../lib/i18n'
import { useSettingsDraft } from '../../composables/useSettingsDraft'

const draft = useSettingsDraft()

const autostart = ref(false)
const error = ref('')

const accountOptions = computed<SelectOption<string | null>[]>(() => [
  { label: t('settings.noAccount'), value: null },
  ...(overview.value?.accounts ?? []).map((account) => ({ label: account.name, value: account.name })),
])

onMounted(async () => {
  try {
    autostart.value = await invoke<boolean>('autostart_enabled')
  } catch (err) {
    error.value = String(err)
  }
})

/**
 * 开机自启是系统状态，不进配置文件，所以不跟着 draft 走：
 * 点一下就地调后端，失败直接把开关拨回去。
 */
async function toggleAutostart() {
  try {
    autostart.value = await invoke<boolean>('set_autostart', { enabled: autostart.value })
    showToast(autostart.value ? t('settings.autostartOn') : t('settings.autostartOff'))
  } catch (err) {
    error.value = String(err)
    autostart.value = !autostart.value
  }
}
</script>

<template>
  <div class="card block">
    <h2>{{ t('settings.defaults') }}</h2>
    <div class="row wrap">
      <div class="field" style="min-width: 200px">
        <label>{{ t('settings.activeAccount') }}</label>
        <SelectInput v-model="draft.activeAccount" :options="accountOptions" />
      </div>
      <label class="switch">
        <input v-model="draft.routes.autoConnect" type="checkbox" />
        <span>{{ t('settings.autoConnect') }}</span>
      </label>
      <label class="switch">
        <input v-model="draft.startMinimized" type="checkbox" />
        <span>{{ t('settings.startMinimized') }}</span>
      </label>
      <label class="switch">
        <input v-model="draft.notifications" type="checkbox" />
        <span>{{ t('settings.notifications') }}</span>
      </label>
    </div>
    <div class="row" style="margin-top: 10px">
      <label class="switch">
        <input v-model="autostart" type="checkbox" @change="toggleAutostart" />
        <span>{{ t('settings.autostart') }}</span>
      </label>
    </div>
    <div class="row" style="margin-top: 10px; gap: 18px">
      <label class="switch">
        <input v-model="draft.startClientOnLaunch" type="checkbox" />
        <span>{{ t('settings.startClient') }}</span>
      </label>
      <label class="switch">
        <input v-model="draft.stopClientOnExit" type="checkbox" />
        <span>{{ t('settings.stopClient') }}</span>
      </label>
    </div>
    <p class="hint" style="margin: 8px 0 0">
      {{ t('settings.clientHint') }}
    </p>
    <p v-if="error" class="error">{{ error }}</p>
  </div>
</template>
