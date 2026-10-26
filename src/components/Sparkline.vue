<script setup lang="ts">
import { computed } from 'vue'
import { t } from '../lib/i18n'

const props = defineProps<{
  values: number[]
  tone?: 'accent' | 'ok'
}>()

const WIDTH = 240
const HEIGHT = 34

const path = computed(() => {
  const values = props.values.slice(-40)
  if (values.length < 2) return ''
  const max = Math.max(...values, 1)
  const step = WIDTH / Math.max(values.length - 1, 1)
  return values
    .map((value, index) => {
      const x = index * step
      const y = HEIGHT - (value / max) * (HEIGHT - 4) - 2
      return `${index === 0 ? 'M' : 'L'}${x.toFixed(1)} ${y.toFixed(1)}`
    })
    .join(' ')
})

const area = computed(() => {
  if (!path.value) return ''
  const values = props.values.slice(-40)
  const step = WIDTH / Math.max(values.length - 1, 1)
  const lastX = (values.length - 1) * step
  return `${path.value} L${lastX.toFixed(1)} ${HEIGHT} L0 ${HEIGHT} Z`
})
</script>

<template>
  <svg
    :viewBox="`0 0 ${WIDTH} ${HEIGHT}`"
    preserveAspectRatio="none"
    class="spark"
    :class="tone ?? 'accent'"
    role="img"
    :aria-label="t('sparkline.aria')"
  >
    <path v-if="area" :d="area" class="area" />
    <path v-if="path" :d="path" class="line" fill="none" stroke-width="1.5" stroke-linejoin="round" />
    <line
      v-if="!path"
      class="base"
      x1="0"
      :y1="HEIGHT - 2"
      :x2="WIDTH"
      :y2="HEIGHT - 2"
      stroke-width="1"
      stroke-dasharray="3 3"
    />
  </svg>
</template>

<style scoped>
.spark {
  width: 100%;
  height: 34px;
  display: block;
  margin-top: 4px;
}

.spark.accent .area {
  fill: var(--accent-soft);
}

.spark.accent .line {
  stroke: var(--accent);
}

.spark.ok .area {
  fill: var(--ok-soft);
}

.spark.ok .line {
  stroke: var(--ok);
}

.base {
  stroke: var(--line-strong);
}
</style>
