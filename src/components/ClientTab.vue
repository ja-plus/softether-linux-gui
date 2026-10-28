<script setup lang="ts">
import { onMounted } from 'vue'
import { RefreshCw } from 'lucide-vue-next'
import { loadSettings, refresh, settings } from '../lib/api'
import { t } from '../lib/i18n'
import { createSettingsDraft, provideSettingsDraft } from '../composables/useSettingsDraft'
import { useAutoSave } from '../composables/useAutoSave'
import ClientDirSettings from './settings/ClientDirSettings.vue'

/**
 * 「SoftEther 客户端」页。
 *
 * 从设置页拆出来单独成页：这一块决定的是**用哪份 vpnclient**，会停/起服务、
 * 会切目录，跟语言、样式这类偏好不是一类东西，埋在设置里最底下没人找得到。
 *
 * 自己持有一份草稿 + 自动保存：卡片是用 provide/inject 拿草稿的，
 * 所以 provide 必须在本组件（卡片的直接父级）完成。
 */

const draft = createSettingsDraft()
provideSettingsDraft(draft)

const { autoState, autoText, persist, fill, markBaseline } = useAutoSave(draft, 'client.autoSaveHint')

onMounted(async () => {
  await loadSettings()
  if (settings.value) {
    fill(settings.value)
    markBaseline(settings.value)
  }
  // 卡片要靠 overview 判断"服务在不在跑、实际跑的是哪个目录"，
  // 而轮询只在「连接」页开着，这里进页面主动读一次，否则显示的是启动时的旧状态。
  void refresh().catch(() => {})
})
</script>

<template>
  <section class="pane">
    <div class="pane-scroll">
      <ClientDirSettings />
    </div>

    <div class="pane-foot">
      <span class="auto-status" :class="autoState">
        <i class="dot" />
        {{ autoText }}
      </span>
      <button v-if="autoState === 'error'" class="ghost" @click="persist">
        <RefreshCw :size="13" /> {{ t('settings.retrySave') }}
      </button>
    </div>
  </section>
</template>
