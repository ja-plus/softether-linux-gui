<script setup lang="ts">
import { onMounted, ref } from 'vue'
import { listen } from '@tauri-apps/api/event'
import { Activity, LayoutList, LogOut, Settings as SettingsIcon } from 'lucide-vue-next'
import OverviewTab from './components/OverviewTab.vue'
import DiagnosticsTab from './components/DiagnosticsTab.vue'
import SettingsTab from './components/SettingsTab.vue'
import TitleBar from './components/TitleBar.vue'
import ResizeEdges from './components/ResizeEdges.vue'
import { bootstrap, overview, toast } from './lib/api'
import { invoke } from '@tauri-apps/api/core'

const tab = ref<'overview' | 'diag' | 'settings'>('overview')
const ready = ref(false)

onMounted(async () => {
  try {
    await bootstrap()
  } finally {
    // 无论数据是否加载成功都要放行主区，否则单条命令失败会导致整页空白
    ready.value = true
  }
  try {
    await listen('open-diagnostics', () => (tab.value = 'diag'))
  } catch {
    // 托盘菜单不可用时仅失去跳转能力，不影响界面
  }
})

async function quit() {
  await invoke('quit_app')
}
</script>

<template>
  <div class="window">
    <TitleBar />
    <div class="shell">
      <aside class="side">
        <nav>
          <button :class="{ active: tab === 'overview' }" @click="tab = 'overview'">
            <LayoutList :size="15" /> 连接
            <span class="pill" :class="overview?.snapshot ? 'ok' : 'off'">{{ overview?.snapshot ? '已连' : '未连' }}</span>
          </button>
          <button :class="{ active: tab === 'diag' }" @click="tab = 'diag'">
            <Activity :size="15" /> 网络诊断
          </button>
          <button :class="{ active: tab === 'settings' }" @click="tab = 'settings'">
            <SettingsIcon :size="15" /> 设置
          </button>
        </nav>
        <div class="side-foot">
          <p class="hint">关闭窗口后仍在后台运行，从托盘图标重新打开。</p>
          <button class="ghost small" @click="quit"><LogOut :size="13" /> 退出应用</button>
        </div>
      </aside>

      <main>
        <OverviewTab v-if="ready && tab === 'overview'" />
        <DiagnosticsTab v-else-if="ready && tab === 'diag'" />
        <SettingsTab v-else-if="ready && tab === 'settings'" />
      </main>

      <div v-if="toast" class="toast" :class="{ error: toast.error }">{{ toast.text }}</div>
    </div>
    <ResizeEdges />
  </div>
</template>

<style scoped>
.window {
  display: flex;
  flex-direction: column;
  height: 100%;
  position: relative;
}

.shell {
  display: grid;
  grid-template-columns: 188px minmax(0, 1fr);
  /* 顶栏固定占 36px，剩余高度才归内容区；不收缩否则内部滚动容器失效 */
  flex: 1;
  min-height: 0;
  position: relative;
}

.side {
  background: var(--panel);
  border-right: 0.5px solid var(--line);
  display: flex;
  flex-direction: column;
  padding: 12px 12px 12px;
  gap: 18px;
}

nav {
  display: flex;
  flex-direction: column;
  gap: 2px;
}

nav button {
  display: flex;
  align-items: center;
  gap: 8px;
  background: none;
  border: none;
  border-radius: var(--radius-ctl);
  padding: 8px 10px;
  color: var(--text-soft);
  text-align: left;
  width: 100%;
  justify-content: flex-start;
  position: relative;
}

nav button:hover {
  background: var(--panel-muted);
}

nav button.active {
  background: var(--nav-active-bg);
  color: var(--nav-active-fg);
}

/* Win11 设置应用的左侧指示条；Deepin 风格下宽度归零，只剩实心胶囊 */
nav button.active::before {
  content: "";
  position: absolute;
  left: 3px;
  top: 50%;
  transform: translateY(-50%);
  width: var(--nav-indicator-w);
  height: 42%;
  border-radius: 999px;
  background: var(--nav-indicator);
}

.pill {
  margin-left: auto;
  font-size: 11px;
  line-height: 16px;
  border-radius: 999px;
  padding: 0 6px;
  border: 0.5px solid transparent;
  flex: none;
  white-space: nowrap;
}

.pill.ok {
  background: var(--ok-soft);
  color: var(--ok);
  border-color: var(--ok-line);
}

.pill.off {
  background: var(--off-bg);
  color: var(--text-hint);
  border-color: var(--off-line);
}

.side-foot {
  margin-top: auto;
  display: flex;
  flex-direction: column;
  gap: 8px;
}

.side-foot button.small {
  display: inline-flex;
  align-items: center;
  gap: 6px;
  justify-content: center;
  min-height: 28px;
  padding: 2px 8px;
  font-size: 12px;
}

main {
  overflow: hidden;
  background: var(--bg);
  /* grid 子项默认 min-height:auto 会按内容撑开，导致内部滚动容器失效 */
  min-height: 0;
}
</style>
