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
import ClientDirSettings from './settings/ClientDirSettings.vue'
import DiagSettings from './settings/DiagSettings.vue'
import InfoTip from './InfoTip.vue'

/**
 * 设置页只负责三件事：造草稿、把草稿发给六张卡片、在底部挂保存状态。
 *
 * 各卡片自己读写草稿里属于自己的字段（provide/inject），
 * 落盘的时机与校验统一由 useAutoSave 决定，卡片不需要知道"保存"这回事。
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
  <section class="settings">
    <div class="settings-scroll">
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
      <ClientDirSettings />
      <DiagSettings />
    </div>

    <div class="foot">
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
.settings {
  display: flex;
  flex-direction: column;
  height: 100%;
  min-height: 0;
}

.settings-scroll {
  flex: 1;
  min-height: 0;
  padding: 18px 20px 24px;
  display: flex;
  flex-direction: column;
  overflow: auto;
  gap: 12px;
}

.title-line {
  display: flex;
  align-items: center;
  gap: 8px;
}

/* 状态栏独立于滚动区，始终固定在设置页底部。 */
.foot {
  flex: none;
  display: flex;
  justify-content: space-between;
  align-items: center;
  gap: 12px;
  padding: 8px 20px 10px;
  background: var(--bg);
  border-top: 0.5px solid var(--line);
}

.auto-status {
  display: inline-flex;
  align-items: center;
  gap: 6px;
  font-size: 12px;
  color: var(--text-hint);
}

.auto-status.saving {
  color: var(--text-soft);
}

.auto-status.saving .dot {
  animation: status-pulse 1s ease-in-out infinite;
}

.auto-status.saved {
  color: var(--ok);
}

.auto-status.blocked {
  color: var(--warn);
}

.auto-status.error {
  color: var(--danger);
}

@keyframes status-pulse {
  50% {
    opacity: 0.2;
  }
}

.foot button {
  display: inline-flex;
  align-items: center;
  gap: 6px;
}
</style>
