<script setup lang="ts">
import { onMounted, ref } from 'vue'
import { listen } from '@tauri-apps/api/event'
import { Activity, LayoutList, Layers, LogOut, Settings as SettingsIcon } from 'lucide-vue-next'
import OverviewTab from './components/OverviewTab.vue'
import ProfilesTab from './components/ProfilesTab.vue'
import DiagnosticsTab from './components/DiagnosticsTab.vue'
import SettingsTab from './components/SettingsTab.vue'
import TitleBar from './components/TitleBar.vue'
import ResizeEdges from './components/ResizeEdges.vue'
import { bootstrap, overview, toast } from './lib/api'
import { t } from './lib/i18n'
import { invoke } from '@tauri-apps/api/core'

const tab = ref<'overview' | 'profiles' | 'diag' | 'settings'>('overview')
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
            <LayoutList :size="15" /> {{ t('app.nav.connections') }}
            <span class="pill" :class="overview?.snapshot ? 'ok' : 'off'">{{ overview?.snapshot ? t('app.badge.connected') : t('app.badge.offline') }}</span>
          </button>
          <button :class="{ active: tab === 'profiles' }" @click="tab = 'profiles'">
            <Layers :size="15" /> {{ t('app.nav.profiles') }}
          </button>
          <button :class="{ active: tab === 'diag' }" @click="tab = 'diag'">
            <Activity :size="15" /> {{ t('app.nav.diag') }}
          </button>
          <button :class="{ active: tab === 'settings' }" @click="tab = 'settings'">
            <SettingsIcon :size="15" /> {{ t('app.nav.settings') }}
          </button>
        </nav>
        <div class="side-foot">
          <button class="ghost small" @click="quit"><LogOut :size="13" /> {{ t('app.quit') }}</button>
        </div>
      </aside>

      <main>
        <OverviewTab v-if="ready && tab === 'overview'" />
        <ProfilesTab v-else-if="ready && tab === 'profiles'" />
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
main {
  overflow: auto;
}

.shell {
  display: flex;
  /* 顶栏固定占 36px，剩余高度才归内容区；不收缩否则内部滚动容器失效 */
  flex: 1;
  min-height: 0;
  position: relative;
}

.side {
  background: var(--side-bg);
  border-right: 0.5px solid var(--line);
  display: flex;
  flex-direction: column;
  padding: 12px 12px 12px;
  gap: 18px;
  flex: 0 0 188px;
  min-height: 0;
  overflow-y: auto;
}

nav {
  display: flex;
  flex-direction: column;
  gap: 2px;
  flex: 0 0 auto;
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
  flex: 0 0 auto;
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
  flex: 1 1 auto;
  min-width: 0;
  min-height: 0;
  display: flex;
  flex-direction: column;
  overflow: hidden;
  /* 给页面滚动区留出与标题栏、窗口边缘的呼吸空间。
     各页自己的 overflow:auto 现在从这里开始，滚动条不会再贴着顶部菜单。 */
  padding: 12px 12px 12px 0;
  background: var(--bg);
}
</style>
