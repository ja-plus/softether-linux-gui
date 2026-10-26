<script setup lang="ts">
import { onBeforeUnmount, onMounted, ref } from 'vue'
import { getCurrentWindow } from '@tauri-apps/api/window'
import { Copy, Minus, Square, X } from 'lucide-vue-next'
import { t } from '../lib/i18n'

const maximized = ref(false)
let unlisten: (() => void) | null = null

onMounted(async () => {
  try {
    const win = getCurrentWindow()
    maximized.value = await win.isMaximized()
    unlisten = await win.onResized(async () => {
      maximized.value = await win.isMaximized()
    })
  } catch {
    // 纯浏览器预览下没有 Tauri 窗口对象，最大化图标停在初始态即可
  }
})

onBeforeUnmount(() => unlisten?.())

/** 窗口按钮统一出口：非 Tauri 环境静默无副作用，真机失败也不弹错误框 */
function withWindow(action: (win: ReturnType<typeof getCurrentWindow>) => Promise<unknown>) {
  try {
    void Promise.resolve(action(getCurrentWindow())).catch(() => {})
  } catch {
    // getCurrentWindow 在非 Tauri 环境会同步抛错
  }
}

function inCaptionButton(target: EventTarget | null): boolean {
  return target instanceof Element && !!target.closest('button')
}

function dragStart(event: MouseEvent) {
  if (event.button !== 0 || inCaptionButton(event.target)) return
  withWindow((win) => win.startDragging())
}

function toggleMaximize(event: MouseEvent) {
  if (inCaptionButton(event.target)) return
  withWindow((win) => win.toggleMaximize())
}
</script>

<template>
  <header class="titlebar" @mousedown="dragStart" @dblclick="toggleMaximize">
    <span class="mark">SE</span>
    <span class="title">SoftEther Tray</span>
    <div class="caption">
      <button class="cap-btn" :aria-label="t('titlebar.minimize')" @click="withWindow((win) => win.minimize())">
        <Minus :size="13" :stroke-width="1.6" />
      </button>
      <button
        class="cap-btn"
        :aria-label="maximized ? t('titlebar.restore') : t('titlebar.maximize')"
        @click="withWindow((win) => win.toggleMaximize())"
      >
        <Copy v-if="maximized" :size="11" :stroke-width="1.6" />
        <Square v-else :size="11" :stroke-width="1.6" />
      </button>
      <button class="cap-btn close" :aria-label="t('titlebar.close')" @click="withWindow((win) => win.close())">
        <X :size="13" :stroke-width="1.6" />
      </button>
    </div>
  </header>
</template>

<style scoped>
.titlebar {
  display: flex;
  align-items: center;
  gap: 8px;
  height: 36px;
  padding-left: 12px;
  background: var(--titlebar-bg);
  border-bottom: 0.5px solid var(--titlebar-border);
  flex: none;
  cursor: default;
}

.mark {
  width: 22px;
  height: 22px;
  border-radius: calc(var(--mark-radius) * 0.7);
  background: var(--mark-bg);
  color: #fff;
  display: flex;
  align-items: center;
  justify-content: center;
  font-size: 9px;
  font-weight: 500;
  letter-spacing: 0.4px;
  flex: none;
}

.title {
  font-size: 12.5px;
  font-weight: 500;
  color: var(--titlebar-fg);
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}

.caption {
  order: var(--caption-order);
  margin: var(--caption-margin);
  display: flex;
  align-items: center;
  gap: var(--caption-gap);
  height: 100%;
  padding-right: var(--caption-gap);
  flex: none;
}

.cap-btn {
  width: var(--caption-w);
  height: var(--caption-h);
  display: flex;
  align-items: center;
  justify-content: center;
  background: var(--caption-dot-min);
  border: var(--caption-border);
  border-radius: var(--caption-radius);
  color: var(--caption-fg);
  padding: 0;
}

/*
 * 每个按钮的底色/hover 色都由风格变量给：
 * Windows/Deepin 平时透明（图标可见）、悬停才出色块；macOS 平时就是红黄绿圆点，
 * 悬停只让图标浮现。这样组件里一个风格判断都不用写 ——
 * 也别用 :global(html[data-style=...]) 做特例，它会把后代选择器整段吞掉。
 */
.cap-btn:nth-child(2) {
  background: var(--caption-dot-max);
}

.cap-btn.close {
  background: var(--caption-dot-close);
}

/* 交通灯只有 12px，图标要缩进圆点里才不溢出（其余风格下 scale 为 1） */
.cap-btn svg {
  transform: scale(var(--caption-icon-scale));
}

.cap-btn:hover {
  background: var(--caption-dot-min-hover);
  color: var(--caption-hover-fg);
}

.cap-btn:nth-child(2):hover {
  background: var(--caption-dot-max-hover);
}

.cap-btn.close:hover {
  background: var(--caption-dot-close-hover);
  color: var(--caption-close-fg);
}
</style>
