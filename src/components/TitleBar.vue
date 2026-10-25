<script setup lang="ts">
import { onBeforeUnmount, onMounted, ref } from 'vue'
import { getCurrentWindow } from '@tauri-apps/api/window'
import { Copy, Minus, Square, X } from 'lucide-vue-next'

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
      <button class="cap-btn" aria-label="最小化" @click="withWindow((win) => win.minimize())">
        <Minus :size="13" :stroke-width="1.6" />
      </button>
      <button
        class="cap-btn"
        :aria-label="maximized ? '还原' : '最大化'"
        @click="withWindow((win) => win.toggleMaximize())"
      >
        <Copy v-if="maximized" :size="11" :stroke-width="1.6" />
        <Square v-else :size="11" :stroke-width="1.6" />
      </button>
      <button class="cap-btn close" aria-label="关闭" @click="withWindow((win) => win.close())">
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
  display: grid;
  place-items: center;
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
  margin-left: auto;
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
  display: grid;
  place-items: center;
  background: transparent;
  border: none;
  border-radius: var(--caption-radius);
  color: var(--caption-fg);
  padding: 0;
}

.cap-btn:hover {
  background: var(--caption-hover-bg);
  color: var(--caption-hover-fg);
}

.cap-btn.close:hover {
  background: var(--caption-close-bg);
  color: var(--caption-close-fg);
}
</style>
