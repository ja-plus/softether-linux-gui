<script setup lang="ts">
import { CircleHelp } from 'lucide-vue-next'

/** 页头说明：平时只留一个问号，悬浮或聚焦时才展开全文 */
defineProps<{ text: string }>()
</script>

<template>
  <span class="info-tip" tabindex="0" :aria-label="text">
    <CircleHelp :size="14" class="icon" aria-hidden="true" />
    <span class="bubble" role="tooltip">{{ text }}</span>
  </span>
</template>

<style scoped>
.info-tip {
  position: relative;
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

.bubble {
  position: absolute;
  top: calc(100% + 8px);
  left: 0;
  z-index: 20;
  width: max-content;
  max-width: 340px;
  padding: 8px 10px;
  background: var(--panel);
  border: 0.5px solid var(--line-strong);
  border-radius: var(--radius);
  box-shadow: var(--shadow-pop);
  color: var(--text-soft);
  font-size: 12px;
  font-weight: 400;
  line-height: 1.55;
  text-align: left;
  white-space: normal;
  opacity: 0;
  visibility: hidden;
  pointer-events: none;
  transform: translateY(-2px);
  transition: opacity 0.12s ease, transform 0.12s ease;
}

.info-tip:hover .bubble,
.info-tip:focus-visible .bubble {
  opacity: 1;
  visibility: visible;
  transform: translateY(0);
}
</style>
