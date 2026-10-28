<script setup lang="ts">
import { onMounted, ref } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import { RefreshCw } from 'lucide-vue-next'
import { loadSettings, settings } from '../lib/api'
import { t } from '../lib/i18n'
import { createSettingsDraft, provideSettingsDraft } from '../composables/useSettingsDraft'
import { useAutoSave } from '../composables/useAutoSave'
import LanguageSettings from './settings/LanguageSettings.vue'
import StyleSettings from './settings/StyleSettings.vue'
import DefaultsSettings from './settings/DefaultsSettings.vue'
import RoutesSettings from './settings/RoutesSettings.vue'
import DiagSettings from './settings/DiagSettings.vue'
import InfoTip from './InfoTip.vue'

/**
 * 设置页只负责三件事：造草稿、把草稿发给五张卡片、在底部挂保存状态。
 *
 * 各卡片自己读写草稿里属于自己的字段（provide/inject），
 * 落盘的时机与校验统一由 useAutoSave 决定，卡片不需要知道"保存"这回事。
 *
 * 注意：`provideSettingsDraft` 必须留在**本组件**，卡片是它的直接子组件。
 * 别把这套壳抽成带 slot 的公共组件——provide/inject 要跨 slot 边界，
 * 那是另一条不稳定的路径，为省几十行模板不值得。
 */

const draft = createSettingsDraft()
provideSettingsDraft(draft)

const { autoState, autoText, persist, fill, markBaseline } = useAutoSave(draft)

const configDir = ref('')

onMounted(async () => {
  await loadSettings()
  if (settings.value) {
    fill(settings.value)
    // 基准值要在回填之后再取，否则第一次改动会和空指纹比较
    markBaseline(settings.value)
  }
  configDir.value = await invoke<string>('config_dir_path')
})
</script>

<template>
  <section class="pane">
    <div class="pane-scroll">
      <header>
        <div class="title-line">
          <h1>{{ t('settings.title') }}</h1>
          <InfoTip :text="t('settings.subtitle', { dir: configDir })" />
        </div>
      </header>

      <LanguageSettings />
      <StyleSettings />
      <DefaultsSettings />
      <RoutesSettings />
      <DiagSettings />
    </div>

    <div class="pane-foot">
      <span class="auto-status" :class="autoState">
        <i class="dot" />
        {{ autoText }}
      </span>
      <!-- 写盘失败时给一个明确的重试入口：改别处也能重试，但那太隐蔽了 -->
      <button v-if="autoState === 'error'" class="ghost" @click="persist">
        <RefreshCw :size="13" /> {{ t('settings.retrySave') }}
      </button>
    </div>
  </section>
</template>

<style scoped>
.title-line {
  display: flex;
  align-items: center;
  gap: 8px;
}
</style>
