<script lang="ts">
export interface SelectOption<T> {
  label: string
  value: T
  disabled?: boolean
}
</script>

<script setup lang="ts" generic="T">
import { computed, nextTick, onBeforeUnmount, ref } from 'vue'
import { Check, ChevronDown } from 'lucide-vue-next'

const props = defineProps<{
  options: SelectOption<T>[]
  placeholder?: string
  disabled?: boolean
}>()

const model = defineModel<T>({ required: true })

const trigger = ref<HTMLElement | null>(null)
const pop = ref<HTMLElement | null>(null)
const open = ref(false)
const activeIndex = ref(-1)
const popStyle = ref<Record<string, string>>({})

const currentLabel = computed(() => {
  const match = props.options.find((option) => option.value === model.value)
  if (match) return match.label
  if (isEmpty(model.value)) return props.placeholder ?? '请选择'
  return String(model.value)
})

const showPlaceholder = computed(
  () => isEmpty(model.value) && !props.options.some((option) => option.value === model.value),
)

function isEmpty(value: unknown): boolean {
  return value === null || value === undefined || value === ''
}

function selectedIndex(): number {
  return props.options.findIndex((option) => option.value === model.value)
}

/** 弹层用 fixed 传送到 body：放在表单里也不会被对话框的滚动容器裁掉 */
function positionPop() {
  const rect = trigger.value?.getBoundingClientRect()
  if (!rect) return
  const gap = 4
  const margin = 8
  const below = window.innerHeight - rect.bottom - gap - margin
  const above = rect.top - gap - margin
  // 下方放不下就翻到上方，两头都紧时取空间大的一侧，条目在弹层内部滚动
  const flip = below < 140 && above > below
  const max = Math.max(120, Math.min(248, flip ? above : below))
  popStyle.value = {
    left: `${rect.left}px`,
    width: `${rect.width}px`,
    'max-height': `${max}px`,
    ...(flip ? { bottom: `${window.innerHeight - rect.top + gap}px` } : { top: `${rect.bottom + gap}px` }),
  }
}

function onPointerDown(event: PointerEvent) {
  const target = event.target as Node | null
  if (!target) return
  if (trigger.value?.contains(target) || pop.value?.contains(target)) return
  close()
}

function openMenu() {
  if (props.disabled) return
  open.value = true
  const selected = selectedIndex()
  activeIndex.value = selected >= 0 ? selected : firstEnabled()
  nextTick(positionPop)
  document.addEventListener('pointerdown', onPointerDown, true)
  // 滚动/缩放时跟着触发器走，而不是关掉：对话框滚动条动一下就消失很恼人
  window.addEventListener('scroll', positionPop, true)
  window.addEventListener('resize', positionPop)
}

function close() {
  if (!open.value) return
  open.value = false
  document.removeEventListener('pointerdown', onPointerDown, true)
  window.removeEventListener('scroll', positionPop, true)
  window.removeEventListener('resize', positionPop)
}

onBeforeUnmount(close)

function firstEnabled(): number {
  return props.options.findIndex((option) => !option.disabled)
}

// 项目 tsconfig 目标是 es2021，没有 findLastIndex，用倒序循环
function lastEnabled(): number {
  for (let i = props.options.length - 1; i >= 0; i--) {
    if (!props.options[i].disabled) return i
  }
  return -1
}

function moveActive(step: number) {
  const count = props.options.length
  if (!count) return
  let index = activeIndex.value
  for (let i = 0; i < count; i++) {
    index = (index + step + count) % count
    if (!props.options[index].disabled) break
  }
  activeIndex.value = index
  scrollActiveIntoView()
}

function scrollActiveIntoView() {
  nextTick(() => {
    pop.value?.querySelector('[data-active="true"]')?.scrollIntoView({ block: 'nearest' })
  })
}

function pick(index: number) {
  const option = props.options[index]
  if (!option || option.disabled) return
  model.value = option.value
  close()
  trigger.value?.focus()
}

function onKeydown(event: KeyboardEvent) {
  if (props.disabled) return
  if (!open.value) {
    if (['ArrowDown', 'ArrowUp', 'Enter', ' '].includes(event.key)) {
      event.preventDefault()
      openMenu()
    }
    return
  }
  switch (event.key) {
    case 'ArrowDown':
      event.preventDefault()
      moveActive(1)
      break
    case 'ArrowUp':
      event.preventDefault()
      moveActive(-1)
      break
    case 'Home':
      event.preventDefault()
      activeIndex.value = firstEnabled()
      scrollActiveIntoView()
      break
    case 'End':
      event.preventDefault()
      activeIndex.value = lastEnabled()
      scrollActiveIntoView()
      break
    case 'Enter':
    case ' ':
      event.preventDefault()
      pick(activeIndex.value)
      break
    case 'Escape':
      event.preventDefault()
      close()
      break
    case 'Tab':
      close()
      break
  }
}
</script>

<template>
  <div class="select">
    <button
      ref="trigger"
      type="button"
      class="trigger"
      :class="{ open, placeholder: showPlaceholder }"
      :disabled="disabled"
      aria-haspopup="listbox"
      :aria-expanded="open"
      @click="open ? close() : openMenu()"
      @keydown="onKeydown"
    >
      <span class="label">{{ currentLabel }}</span>
      <ChevronDown :size="14" class="chevron" />
    </button>
    <Teleport to="body">
      <div v-if="open" ref="pop" class="select-pop" role="listbox" :style="popStyle" @mousedown.prevent>
        <button
          v-for="(option, index) in options"
          :key="`${index}-${option.label}`"
          type="button"
          class="option"
          role="option"
          :class="{ selected: option.value === model, active: index === activeIndex }"
          :data-active="index === activeIndex"
          :aria-selected="option.value === model"
          :disabled="option.disabled"
          @mouseenter="!option.disabled && (activeIndex = index)"
          @click="pick(index)"
        >
          <span class="option-label">{{ option.label }}</span>
          <Check v-if="option.value === model" :size="14" class="check" />
        </button>
        <p v-if="!options.length" class="empty">{{ placeholder ?? '暂无可选项' }}</p>
      </div>
    </Teleport>
  </div>
</template>

<style scoped>
.select {
  position: relative;
  width: 100%;
}

.trigger {
  display: flex;
  align-items: center;
  gap: 8px;
  width: 100%;
  min-height: 32px;
  padding: 6px 10px;
  background: var(--panel);
  border: 0.5px solid var(--line-strong);
  border-radius: var(--radius-ctl);
  color: var(--text);
  text-align: left;
}

.trigger:hover:not(:disabled) {
  border-color: var(--text-hint);
}

.trigger:disabled {
  opacity: 0.45;
  cursor: default;
}

.trigger.open {
  border-color: var(--accent);
  box-shadow: 0 0 0 3px var(--focus-ring);
}

.label {
  flex: 1;
  min-width: 0;
  overflow: hidden;
  white-space: nowrap;
  text-overflow: ellipsis;
}

.trigger.placeholder .label {
  color: var(--text-hint);
}

.chevron {
  flex: none;
  color: var(--text-hint);
  transition: transform 0.12s ease;
}

.trigger.open .chevron {
  transform: rotate(180deg);
}

.select-pop {
  position: fixed;
  z-index: 60;
  overflow: auto;
  padding: 4px;
  background: var(--panel);
  border: 0.5px solid var(--line);
  border-radius: var(--radius);
  box-shadow: var(--shadow-pop);
}

.option {
  display: flex;
  align-items: center;
  gap: 8px;
  width: 100%;
  padding: 7px 10px;
  border: none;
  border-radius: var(--radius-ctl);
  background: transparent;
  color: var(--text);
  font-size: 13px;
  text-align: left;
  cursor: pointer;
}

.option:hover:not(:disabled),
.option.active:not(:disabled) {
  background: var(--ghost-hover-bg);
}

.option.selected {
  background: var(--accent-soft);
  color: var(--accent);
}

.option:disabled {
  opacity: 0.45;
  cursor: default;
  background: transparent;
}

.option-label {
  flex: 1;
  min-width: 0;
  overflow: hidden;
  white-space: nowrap;
  text-overflow: ellipsis;
}

.check {
  flex: none;
}

.empty {
  margin: 0;
  padding: 8px 10px;
  color: var(--text-hint);
  font-size: 12px;
}
</style>
