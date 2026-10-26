<script setup lang="ts">
import { onMounted, ref } from 'vue'
import { listen } from '@tauri-apps/api/event'
import { Activity, Layers, LayoutList, LogOut, Plug, Settings as SettingsIcon } from 'lucide-vue-next'
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
            <span class="pill" :class="overview?.snapshot ? 'ok' : 'off'" :aria-label="overview?.snapshot ? t('app.badge.connected') : t('app.badge.offline')">
              <svg v-if="overview?.snapshot" class="link-state-icon" viewBox="0 0 1024 1024" aria-hidden="true">
                <path d="M935.754805 89.966493a47.62257 47.62257 0 0 0-3.672102-4.590127A252.170113 252.170113 0 0 0 753.698885 8.893371c-66.384715 1.147532-131.794027 24.72931-180.621505 72.35188l-175.055976 175.572365A248.096375 248.096375 0 0 0 325.956407 437.209615a258.538914 258.538914 0 0 0 76.540371 179.875609c1.204908 1.204908 2.52457 2.295064 3.901608 3.270466a68.163389 68.163389 0 0 0 94.72875-4.475374 68.278142 68.278142 0 0 0-2.409817-95.991035 123.187539 123.187539 0 0 1-36.548887-85.720625 112.22861 112.22861 0 0 1 32.30302-81.302628l174.654339-175.342859c22.72113-21.745728 52.384827-34.425954 83.884575-32.589903a116.93349 116.93349 0 0 1 82.851796 36.835771c22.950636 22.72113 35.917745 53.130722 36.548887 85.778002a110.908948 110.908948 0 0 1-31.384994 80.212472L728.625315 456.717656a67.704376 67.704376 0 0 0-20.885079 47.852076c-0.401636 18.245756 6.368801 35.516109 19.049028 48.655348a68.278142 68.278142 0 0 0 96.507425 1.836051l113.376141-109.417157a248.899647 248.899647 0 0 0 72.064997-180.334622c-1.147532-67.933882-28.401412-131.73665-72.983022-175.342859z" />
                <path d="M520.864683 400.259091a68.565025 68.565025 0 0 0-1.893427 97.023814c22.950636 22.778506 35.975122 53.245475 36.606264 85.720625a111.88435 111.88435 0 0 1-32.30302 81.302628l-174.424833 175.055976a121.179358 121.179358 0 0 1-170.752732-171.728134l113.605648-119.974449a68.335519 68.335519 0 0 0-2.52457-96.450048 67.589623 67.589623 0 0 0-48.770101-18.647392c-18.188379 0.459013-35.114473 8.032723-47.737323 21.229339l-112.458116 118.769541c-96.679554 100.638539-94.269737 261.924133 5.106516 359.292206a260.661848 260.661848 0 0 0 183.605088 76.827253h1.204908a244.998039 244.998039 0 0 0 174.31008-72.294503l175.342859-175.629741a249.18653 249.18653 0 0 0 72.122374-180.334623 259.227433 259.227433 0 0 0-77.22889-180.449375 68.507648 68.507648 0 0 0-93.810725 0.286883z" />
              </svg>
              <Plug v-else :size="14" />
            </span>
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
  flex: 1 1 auto;
  min-width: 0;
  min-height: 0;
  display: flex;
  flex-direction: column;
  background: var(--bg);
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
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 24px;
  height: 24px;
  border-radius: 50%;
  border: 0.5px solid transparent;
  flex: none;
}

.link-state-icon {
  width: 14px;
  height: 14px;
  fill: currentColor;
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

</style>
