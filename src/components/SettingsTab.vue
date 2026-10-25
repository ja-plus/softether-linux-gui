<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, reactive, ref, watch } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import { Check, Plus, Save, RotateCcw } from 'lucide-vue-next'
import SelectInput, { type SelectOption } from './SelectInput.vue'
import {
  applyUiStyle,
  loadSettings,
  overview,
  settings,
  showToast,
  type Settings,
  type UiStyle,
} from '../lib/api'

const draft = reactive<Settings>({
  vpnDir: null,
  vpncmdPath: null,
  activeAccount: null,
  notifications: true,
  startMinimized: true,
  diagTarget: '',
  routes: { subnets: ['10.0.0.0/8', '172.16.0.0/12', '192.168.0.0/16'], autoConnect: false, dhcp: true, mtu: 1400 },
  startClientOnLaunch: false,
  stopClientOnExit: false,
  uiStyle: 'win11',
})

const newSubnet = ref('')
const autostart = ref(false)
const configDir = ref('')
const saving = ref(false)
const error = ref('')

const accountOptions = computed<SelectOption<string | null>[]>(() => [
  { label: '（未选择）', value: null },
  ...(overview.value?.accounts ?? []).map((account) => ({ label: account.name, value: account.name })),
])

function fill(source: Settings) {
  Object.assign(draft, {
    ...source,
    routes: { ...source.routes, subnets: [...source.routes.subnets] },
  })
}

onMounted(async () => {
  await loadSettings()
  if (settings.value) fill(settings.value)
  autostart.value = await invoke<boolean>('autostart_enabled')
  configDir.value = await invoke<string>('config_dir_path')
})

watch(settings, (value) => {
  if (value) fill(value)
})

function pickStyle(style: UiStyle) {
  draft.uiStyle = style
  // watch 只在取值变化时触发；点已选中的卡片也要能恢复预览
  applyUiStyle(style)
}

// 风格切换即时预览整套界面，不用等保存
watch(
  () => draft.uiStyle,
  (value) => applyUiStyle(value),
)

// 没保存就切走页面：撤销实时预览，回到已保存的风格
onBeforeUnmount(() => {
  if (settings.value) applyUiStyle(settings.value.uiStyle)
})

function addSubnet() {
  const value = newSubnet.value.trim()
  if (!value) return
  if (!/^\d{1,3}(\.\d{1,3}){3}\/\d{1,2}$/.test(value)) {
    error.value = `网段格式应为 10.0.0.0/8，收到：${value}`
    return
  }
  if (!draft.routes.subnets.includes(value)) draft.routes.subnets.push(value)
  newSubnet.value = ''
  error.value = ''
}

function removeSubnet(value: string) {
  draft.routes.subnets = draft.routes.subnets.filter((item) => item !== value)
}

function resetRoutes() {
  draft.routes.subnets = ['10.0.0.0/8', '172.16.0.0/12', '192.168.0.0/16']
  draft.routes.mtu = 1400
  draft.routes.dhcp = false
}

async function toggleAutostart() {
  try {
    autostart.value = await invoke<boolean>('set_autostart', { enabled: autostart.value })
    showToast(autostart.value ? '已开启开机自启动' : '已关闭开机自启动')
  } catch (err) {
    error.value = String(err)
    autostart.value = !autostart.value
  }
}

async function save() {
  error.value = ''
  saving.value = true
  try {
    const saved = await invoke<Settings>('save_settings', { settings: { ...draft, routes: { ...draft.routes } } })
    fill(saved)
    settings.value = saved
    showToast('设置已保存')
  } catch (err) {
    error.value = String(err)
  } finally {
    saving.value = false
  }
}
</script>

<template>
  <section class="settings">
    <header>
      <h1>设置</h1>
      <p class="hint">设置保存在 <span class="mono">{{ configDir }}</span>，密码不会写进这个文件。</p>
    </header>

    <div class="card block">
      <h2>界面风格</h2>
      <div class="styles">
        <button
          type="button"
          class="style-card"
          :class="{ on: draft.uiStyle === 'win11' }"
          @click="pickStyle('win11')"
        >
          <span class="preview win11">
            <span class="pv-side">
              <i class="pv-nav on"></i>
              <i class="pv-nav"></i>
              <i class="pv-nav"></i>
            </span>
            <span class="pv-main">
              <i class="pv-card"></i>
              <i class="pv-btn"></i>
            </span>
          </span>
          <span class="style-name">风格 1 · Windows 11</span>
          <span class="hint">Fluent 浅灰画布、方角控件、左侧强调指示条</span>
        </button>
        <button
          type="button"
          class="style-card"
          :class="{ on: draft.uiStyle === 'deepin' }"
          @click="pickStyle('deepin')"
        >
          <span class="preview deepin">
            <span class="pv-side">
              <i class="pv-nav on"></i>
              <i class="pv-nav"></i>
              <i class="pv-nav"></i>
            </span>
            <span class="pv-main">
              <i class="pv-card"></i>
              <i class="pv-btn"></i>
            </span>
          </span>
          <span class="style-name">风格 2 · Deepin 25</span>
          <span class="hint">大圆角卡片、蓝色实心导航胶囊、柔和投影</span>
        </button>
      </div>
      <p class="hint" style="margin: 2px 0 0">切换立即预览；确认后点下方「保存设置」，下次启动沿用。</p>
    </div>

    <div class="card block">
      <h2>默认连接</h2>
      <div class="row wrap">
        <div class="field" style="min-width: 200px">
          <label>默认连接配置</label>
          <SelectInput v-model="draft.activeAccount" :options="accountOptions" />
        </div>
        <label class="switch">
          <input v-model="draft.routes.autoConnect" type="checkbox" />
          <span>启动应用后自动连接</span>
        </label>
        <label class="switch">
          <input v-model="draft.startMinimized" type="checkbox" />
          <span>启动后最小化到托盘</span>
        </label>
        <label class="switch">
          <input v-model="draft.notifications" type="checkbox" />
          <span>连接状态桌面通知</span>
        </label>
      </div>
      <div class="row" style="margin-top: 10px">
        <label class="switch">
          <input v-model="autostart" type="checkbox" @change="toggleAutostart" />
          <span>开机自启动（登录桌面后自动运行）</span>
        </label>
      </div>
      <div class="row" style="margin-top: 10px; gap: 18px">
        <label class="switch">
          <input v-model="draft.startClientOnLaunch" type="checkbox" />
          <span>应用启动时一并启动 SoftEther 客户端服务</span>
        </label>
        <label class="switch">
          <input v-model="draft.stopClientOnExit" type="checkbox" />
          <span>应用退出时停止客户端服务</span>
        </label>
      </div>
      <p class="hint" style="margin: 8px 0 0">
        客户端是 root 常驻进程，启停都会弹一次系统密码框。退出时只停止<strong>由本应用拉起</strong>的客户端；
        你自己先开着的不会被关掉。两个都勾上就是「随本应用开则开、关则关」。
      </p>
    </div>

    <div class="card block">
      <div class="row-between">
        <div>
          <h2>隧道与路由</h2>
          <p class="hint">SoftEther 的 Linux 客户端不会自动改路由表，这一层由应用接管；默认路由永远不交给 VPN。</p>
        </div>
        <button class="ghost" @click="resetRoutes"><RotateCcw :size="14" /> 恢复默认</button>
      </div>

      <div class="subnets">
        <span v-for="subnet in draft.routes.subnets" :key="subnet" class="chip mono">
          {{ subnet }}
          <button @click="removeSubnet(subnet)" aria-label="删除">×</button>
        </span>
        <span v-if="!draft.routes.subnets.length" class="hint">还没有网段，连接后不会下发任何路由。</span>
      </div>
      <div class="row" style="margin-top: 8px">
        <input v-model="newSubnet" type="text" placeholder="新增网段，例如 10.20.0.0/16" style="max-width: 240px" @keyup.enter="addSubnet" />
        <button class="ghost" @click="addSubnet"><Plus :size="14" /> 添加</button>
      </div>

      <div class="row wrap" style="margin-top: 12px">
        <div class="field" style="width: 140px">
          <label>隧道 MTU</label>
          <input v-model.number="draft.routes.mtu" type="number" min="576" max="1500" />
        </div>
        <label class="switch">
          <input v-model="draft.routes.dhcp" type="checkbox" />
          <span>断开时清空虚拟网卡的 IPv4 地址</span>
        </label>
      </div>
      <p class="hint" style="margin: 8px 0 0">
        默认 1400 是给 SoftEther 外层封装留余量。只有在“小包正常、大包失败”时才需要往下调。
      </p>
      <p class="hint" style="margin: 6px 0 0">
        连接时若网卡没有 IPv4，应用会自动要一次 DHCP；地址是用
        <span class="mono">dhclient -1</span> 要来的，要完就退出，所以断开时得靠上面这个开关收尾。
        网卡上配的是静态地址就把它关掉，免得被清。
      </p>
    </div>

    <div class="card block">
      <h2>客户端与诊断</h2>
      <div class="row wrap">
        <div class="field" style="flex: 1; min-width: 240px">
          <label>SoftEther 客户端目录</label>
          <input v-model="draft.vpnDir" type="text" placeholder="/home/you/Downloads/vpnclient" />
        </div>
        <div class="field" style="flex: 1; min-width: 240px">
          <label>诊断默认目标</label>
          <input v-model="draft.diagTarget" type="text" placeholder="10.10.4.160" />
        </div>
      </div>
      <p class="hint" style="margin: 8px 0 0">
        当前找到的目录：<span class="mono">{{ overview?.vpnDir ?? '未找到' }}</span>；留空会自动查找
        <span class="mono">~/Downloads/vpnclient</span>、<span class="mono">/usr/local/vpnclient</span>。
      </p>
    </div>

    <p v-if="error" class="error">{{ error }}</p>

    <div class="foot">
      <span class="hint">保存后新连接立即生效；已连接的隧道需要重新连接才会应用新路由。</span>
      <button class="primary" :disabled="saving" @click="save">
        <Save :size="14" /> {{ saving ? '保存中…' : '保存设置' }}
      </button>
    </div>
  </section>
</template>

<style scoped>
.settings {
  padding: 18px 20px 24px;
  display: flex;
  flex-direction: column;
  gap: 12px;
  height: 100%;
  overflow: auto;
}

.block {
  padding: 14px 16px;
  display: flex;
  flex-direction: column;
  gap: 10px;
}

.row-between {
  display: flex;
  justify-content: space-between;
  align-items: flex-start;
  gap: 16px;
}

.row.wrap {
  flex-wrap: wrap;
  gap: 12px 18px;
}

.switch {
  display: inline-flex;
  align-items: center;
  gap: 6px;
  font-size: 13px;
  color: var(--text-soft);
  min-height: 32px;
}

.subnets {
  display: flex;
  flex-wrap: wrap;
  gap: 6px;
}

.chip {
  display: inline-flex;
  align-items: center;
  gap: 6px;
  background: var(--accent-soft);
  color: var(--accent-ink);
  border: 0.5px solid var(--accent-line);
  border-radius: 999px;
  padding: 2px 4px 2px 10px;
}

.chip button {
  background: none;
  border: none;
  color: var(--accent);
  font-size: 14px;
  line-height: 1;
  padding: 0 4px;
}

.foot {
  display: flex;
  justify-content: space-between;
  align-items: center;
  gap: 12px;
}

.foot button,
.block .row button,
.row-between button {
  display: inline-flex;
  align-items: center;
  gap: 6px;
}

.error {
  background: var(--danger-soft);
  color: var(--danger);
  border: 0.5px solid var(--danger-line);
  border-radius: var(--radius-ctl);
  padding: 8px 10px;
  margin: 0;
  font-size: 12px;
}

.styles {
  display: flex;
  flex-wrap: wrap;
  gap: 12px;
}

.style-card {
  display: flex;
  flex-direction: column;
  align-items: flex-start;
  gap: 3px;
  width: 232px;
  padding: 10px;
  background: var(--panel);
  border: 1px solid var(--line-strong);
  border-radius: var(--radius);
  text-align: left;
}

.style-card:hover {
  border-color: var(--accent);
}

.style-card.on {
  border-color: var(--accent);
  box-shadow: 0 0 0 1px var(--accent);
}

.style-name {
  font-weight: 500;
  margin-top: 2px;
}

/* 缩略示意图刻意不跟主题变量走：两块预览各画各的风格，切换前后都得是“目标风格的样子” */
.preview {
  position: relative;
  display: flex;
  width: 100%;
  height: 62px;
  border-radius: 6px;
  overflow: hidden;
  border: 0.5px solid rgba(0, 0, 0, 0.1);
}

.style-card.on .preview::after {
  content: "✓";
  position: absolute;
  top: 5px;
  right: 5px;
  width: 15px;
  height: 15px;
  border-radius: 50%;
  background: var(--accent);
  color: #fff;
  font-size: 10px;
  line-height: 15px;
  text-align: center;
}

.pv-side {
  width: 34%;
  display: flex;
  flex-direction: column;
  gap: 4px;
  padding: 8px 6px;
}

.pv-main {
  flex: 1;
  display: flex;
  flex-direction: column;
  gap: 6px;
  padding: 9px 8px;
}

.pv-nav {
  display: block;
  height: 7px;
}

.pv-card {
  display: block;
  height: 22px;
}

.pv-btn {
  display: block;
  width: 34px;
  height: 11px;
}

.preview.win11 {
  background: #f3f3f3;
}

.preview.win11 .pv-side {
  background: #ffffff;
  border-right: 1px solid #e5e5e5;
}

.preview.win11 .pv-nav {
  background: #e5e5e5;
  border-radius: 2px;
}

.preview.win11 .pv-nav.on {
  background: #e8f1fb;
  border-left: 2px solid #0067c0;
}

.preview.win11 .pv-card {
  background: #ffffff;
  border: 1px solid #e5e5e5;
  border-radius: 3px;
}

.preview.win11 .pv-btn {
  background: #0067c0;
  border-radius: 2px;
}

.preview.deepin {
  background: #eef1f6;
}

.preview.deepin .pv-side {
  background: rgba(255, 255, 255, 0.85);
  border-right: 1px solid #e4e8ee;
}

.preview.deepin .pv-nav {
  background: #dde3ec;
  border-radius: 999px;
}

.preview.deepin .pv-nav.on {
  background: #2b79ff;
}

.preview.deepin .pv-card {
  background: #ffffff;
  border-radius: 6px;
  box-shadow: 0 2px 6px rgba(35, 55, 95, 0.18);
}

.preview.deepin .pv-btn {
  background: #2b79ff;
  border-radius: 5px;
  box-shadow: 0 1px 3px rgba(43, 121, 255, 0.4);
}
</style>
