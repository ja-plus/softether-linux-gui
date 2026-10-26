<script lang="ts">
/*
 * 数字输入框，替代原生 <input type="number">。
 *
 * 为什么不用原生：
 * - WebKit 的 ::-webkit-inner-spin-button 宽约 10px、箭头只有 6px，在 96dpi 的桌面
 *   托盘窗口里又细又难点；而且它没法跟随 --accent / --radius-ctl 这些主题变量。
 * - 原生的滚轮改值在设置面板里极容易误触（滚页面时顺手把 MTU 改了）。
 *
 * 所以这里隐藏原生行为（type="text" + inputmode），自己画右侧步进块：
 * 30px 宽、上下各占一半、hover 铺 --accent-soft，按住可连续步进。
 * 组件只面向非负整数（端口 / MTU / 次数 / 连接数），小数与负号会被过滤掉。
 */
</script>

<script setup lang="ts">
import { computed, onBeforeUnmount, ref, watch } from 'vue'
import { ChevronDown, ChevronUp } from 'lucide-vue-next'
import { t } from '../lib/i18n'

const props = withDefaults(
  defineProps<{
    min?: number
    max?: number
    step?: number
    disabled?: boolean
    /** 长按步进键时的连续步进间隔（ms） */
    repeatInterval?: number
  }>(),
  { step: 1, repeatInterval: 60 },
)

const model = defineModel<number>({ required: true })

/**
 * 输入过程中的原始文本。允许暂时不合法（清空、只打了半截），
 * 只有失焦 / 回车 / 点步进键时才归一化并回写 model。
 */
const text = ref(String(model.value ?? ''))
const editing = ref(false)

watch(model, (value) => {
  if (editing.value) return
  text.value = String(value ?? '')
})

const atMin = computed(() => props.min !== undefined && Number(model.value) <= props.min)
const atMax = computed(() => props.max !== undefined && Number(model.value) >= props.max)

function clamp(value: number): number {
  let next = Number.isFinite(value) ? Math.round(value) : (props.min ?? 0)
  if (props.min !== undefined) next = Math.max(props.min, next)
  if (props.max !== undefined) next = Math.min(props.max, next)
  return next
}

function inRange(value: number): boolean {
  if (props.min !== undefined && value < props.min) return false
  if (props.max !== undefined && value > props.max) return false
  return true
}

/** 把当前文本落成合法数字并回写，返回落盘后的值 */
function commit(): number {
  const digits = text.value.replace(/[^\d]/g, '')
  const next = clamp(digits === '' ? (props.min ?? 0) : Number(digits))
  model.value = next
  text.value = String(next)
  return next
}

function onInput(event: Event) {
  const el = event.target as HTMLInputElement
  // 先清洗再写回 DOM：粘贴 "1 500" 之类也能得到 1500，中文字符直接丢掉
  const cleaned = el.value.replace(/[^\d]/g, '')
  if (el.value !== cleaned) el.value = cleaned
  text.value = cleaned

  // 已经落在区间内的值实时同步给父组件，让「保存」按钮的可用状态不必等失焦；
  // 越界的中间态（打 1500 时的 "1"）先不动，否则会被 clamp 打断输入。
  const value = Number(cleaned)
  if (cleaned !== '' && inRange(value)) model.value = value
}

function step(dir: 1 | -1) {
  // 别只靠按钮的 :disabled —— 禁用的按钮仍可能被指针事件命中原生 handler，
  // 这里兜一道，保证禁用态和已到边界时绝不写值
  if (props.disabled) return
  if (dir === 1 ? atMax.value : atMin.value) return
  // 先归一当前文本：用户可能刚打完字就点箭头，此时 text 还是脏的
  const next = clamp(commit() + dir * props.step)
  model.value = next
  text.value = String(next)
}

/*
 * 长按连续步进：pointerdown 立刻走一步，停 380ms 后开始每 60ms 一步。
 * 端口要从 443 调到 8443 时逐下点击太累。
 */
let holdTimeout: number | undefined
let holdInterval: number | undefined

function clearHold() {
  if (holdTimeout !== undefined) window.clearTimeout(holdTimeout)
  if (holdInterval !== undefined) window.clearInterval(holdInterval)
  holdTimeout = undefined
  holdInterval = undefined
}

function startHold(dir: 1 | -1) {
  clearHold()
  step(dir)
  holdTimeout = window.setTimeout(() => {
    holdInterval = window.setInterval(() => step(dir), props.repeatInterval)
  }, 380)
}

/** 键盘激活（Enter/Space）的 click 里 detail 为 0，鼠标的已被 pointerdown 处理过 */
function onClick(event: MouseEvent, dir: 1 | -1) {
  if (event.detail === 0) step(dir)
}

function onKeydown(event: KeyboardEvent) {
  if (event.key === 'ArrowUp') {
    event.preventDefault()
    step(1)
  } else if (event.key === 'ArrowDown') {
    event.preventDefault()
    step(-1)
  } else if (event.key === 'Enter') {
    event.preventDefault()
    commit()
    ;(event.target as HTMLInputElement).blur()
  }
}

function onFocus() {
  editing.value = true
}

function onBlur() {
  editing.value = false
  commit()
}

onBeforeUnmount(clearHold)
</script>

<template>
  <div class="number-input" :class="{ disabled }">
    <input
      class="input"
      type="text"
      inputmode="numeric"
      autocomplete="off"
      spellcheck="false"
      :value="text"
      :disabled="disabled"
      @input="onInput"
      @focus="onFocus"
      @blur="onBlur"
      @keydown="onKeydown"
    />
    <div class="stepper">
      <button
        type="button"
        class="step"
        tabindex="-1"
        :disabled="disabled || atMax"
        :aria-label="t('common.increase')"
        @pointerdown.prevent="startHold(1)"
        @pointerup="clearHold"
        @pointerleave="clearHold"
        @pointercancel="clearHold"
        @click="onClick($event, 1)"
      >
        <ChevronUp :size="13" :stroke-width="2.4" />
      </button>
      <button
        type="button"
        class="step"
        tabindex="-1"
        :disabled="disabled || atMin"
        :aria-label="t('common.decrease')"
        @pointerdown.prevent="startHold(-1)"
        @pointerup="clearHold"
        @pointerleave="clearHold"
        @pointercancel="clearHold"
        @click="onClick($event, -1)"
      >
        <ChevronDown :size="13" :stroke-width="2.4" />
      </button>
    </div>
  </div>
</template>

<style scoped>
/*
 * 边框与聚焦环都画在容器上，input 自己无边框无背景：
 * 这样步进块和输入区共用一条外轮廓，不会出现两层圆角打架。
 */
.number-input {
  display: flex;
  align-items: stretch;
  width: 100%;
  min-height: 34px;
  background: var(--panel);
  border: 0.5px solid var(--line-strong);
  border-radius: var(--radius-ctl);
  overflow: hidden;
  transition: border-color 0.12s ease, box-shadow 0.12s ease;
}

.number-input:hover:not(.disabled) {
  border-color: var(--text-hint);
}

/*
 * 聚焦态要压过 hover：鼠标按在步进块上时控件同时处于 hover 和 focus-within，
 * 两者特异性相同，靠源码顺序让聚焦的 accent 描边胜出。
 */
.number-input:focus-within:not(.disabled) {
  border-color: var(--accent);
  box-shadow: 0 0 0 3px var(--focus-ring);
}

.number-input.disabled {
  opacity: 0.45;
}

.input {
  flex: 1;
  min-width: 0;
  /* 全局 input[type=text] 带 min-height:34px，这里必须清零，不然会顶出 36px 的控件 */
  min-height: 0;
  padding: 6px 8px 6px 10px;
  border: none;
  border-radius: 0;
  background: transparent;
  color: var(--text);
  /* 等宽数字：步进时字宽不跳 */
  font-variant-numeric: tabular-nums;
}

/* 压掉全局 input:focus 的描边和环，改由容器 :focus-within 表达 */
.input:focus {
  border-color: transparent;
  box-shadow: none;
}

.input:disabled {
  cursor: default;
}

.stepper {
  display: flex;
  flex-direction: column;
  flex: none;
  width: 30px;
  /* 用画布色而不是 panel-muted：后者和白底只差一点，96dpi 下看不出「这是一块按钮」 */
  border-left: 0.5px solid var(--line-strong);
  background: var(--bg);
}

.step {
  flex: 1;
  display: grid;
  place-items: center;
  min-height: 0;
  padding: 0;
  border: none;
  background: transparent;
  color: var(--text-soft);
  cursor: pointer;
  transition: background 0.1s ease, color 0.1s ease;
}

.step + .step {
  border-top: 0.5px solid var(--line-strong);
}

.step:hover:not(:disabled) {
  background: var(--accent-soft);
  color: var(--accent);
}

.step:active:not(:disabled) {
  background: var(--accent-line);
}

.step:disabled {
  opacity: 0.3;
  cursor: default;
}
</style>
