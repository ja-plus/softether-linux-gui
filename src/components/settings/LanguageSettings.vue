<script setup lang="ts">
import { watch } from 'vue'
import { Check } from 'lucide-vue-next'
import { applyLocale, t, type Locale } from '../../lib/i18n'
import { useSettingsDraft } from '../../composables/useSettingsDraft'

const draft = useSettingsDraft()

/** 直接改草稿，由下面的 watch 统一落到界面上——单一数据流，回填时也能生效 */
function pick(value: Locale) {
  draft.locale = value
}

watch(
  () => draft.locale,
  (value) => applyLocale(value as Locale),
)
</script>

<template>
  <div class="card block">
    <h2>{{ t('settings.language') }}</h2>
    <div class="langs">
      <button type="button" class="lang-card" :class="{ on: draft.locale === 'zh' }" @click="pick('zh')">
        <span class="radio" :class="{ on: draft.locale === 'zh' }">
          <Check v-if="draft.locale === 'zh'" :size="11" />
        </span>
        {{ t('settings.languageZh') }}
      </button>
      <button type="button" class="lang-card" :class="{ on: draft.locale === 'en' }" @click="pick('en')">
        <span class="radio" :class="{ on: draft.locale === 'en' }">
          <Check v-if="draft.locale === 'en'" :size="11" />
        </span>
        {{ t('settings.languageEn') }}
      </button>
    </div>
    <p class="hint" style="margin: 2px 0 0">{{ t('settings.languageHint') }}</p>
  </div>
</template>

<style scoped>
.langs {
  display: flex;
  flex-wrap: wrap;
  gap: 8px;
}

.lang-card {
  display: inline-flex;
  align-items: center;
  gap: 7px;
  min-width: 132px;
  padding: 7px 12px;
  background: var(--panel);
  border: 1px solid var(--line-strong);
  border-radius: var(--radius-ctl);
  color: var(--text);
  text-align: left;
}

.lang-card:hover {
  border-color: var(--accent);
}

.lang-card.on {
  border-color: var(--accent);
  background: var(--accent-soft);
  color: var(--accent-ink);
}
</style>
