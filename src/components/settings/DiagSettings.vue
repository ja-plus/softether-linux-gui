<script setup lang="ts">
import { t } from '../../lib/i18n'
import { useSettingsDraft } from '../../composables/useSettingsDraft'

const draft = useSettingsDraft()
</script>

<template>
  <div class="card block">
    <h2>{{ t('client.diagTitle') }}</h2>
    <div class="field" style="max-width: 320px">
      <label>{{ t('client.diagTargetLabel') }}</label>
      <input v-model="draft.diagTarget" type="text" :placeholder="t('diag.targetPlaceholder')" />
    </div>
    <p class="hint" style="margin: 4px 0 0">{{ t('client.diagHint') }}</p>
  </div>
</template>
