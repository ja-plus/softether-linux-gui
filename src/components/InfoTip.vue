<script setup lang="ts">
import { CircleHelp } from 'lucide-vue-next'
import { Tooltip as ATooltip } from '@arco-design/web-vue'
import '@arco-design/web-vue/es/tooltip/style/css.js'

/** 页头说明：平时只留一个问号，悬浮时由 Arco Tooltip 展开全文（自动定位、自动翻转） */
defineProps<{ text: string }>()
</script>

<template>
  <ATooltip :content="text" position="bottom">
    <span class="info-tip" tabindex="0" :aria-label="text">
      <CircleHelp :size="14" class="icon" aria-hidden="true" />
    </span>
  </ATooltip>
</template>

<style scoped>
.info-tip {
  display: inline-flex;
  align-items: center;
  align-self: center;
  color: var(--text-hint);
  cursor: help;
  outline: none;
}

.icon {
  display: block;
}

.info-tip:hover,
.info-tip:focus-visible {
  color: var(--text-soft);
}
</style>
