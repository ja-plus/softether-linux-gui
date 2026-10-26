<script setup lang="ts">
import { computed, nextTick, onBeforeUnmount, onMounted, reactive, ref, watch } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import { Check, Plus, RefreshCw, RotateCcw, ShieldCheck, TriangleAlert } from 'lucide-vue-next'
import SelectInput, { type SelectOption } from './SelectInput.vue'
import NumberInput from './NumberInput.vue'
import {
  applyUiStyle,
  loadSettings,
  overview,
  settings,
  showToast,
  type ClientCandidate,
  type ClientSource,
  type Settings,
  type UiStyle,
} from '../lib/api'
import { applyLocale, locale, t, type Locale } from '../lib/i18n'

const draft = reactive<Settings>({
  vpnDir: null,
  clientSource: 'bundled',
  vpncmdPath: null,
  activeAccount: null,
  notifications: true,
  startMinimized: true,
  diagTarget: '',
  routes: { subnets: ['10.0.0.0/8', '172.16.0.0/12', '192.168.0.0/16'], autoConnect: false, dhcp: true, mtu: 1400 },
  startClientOnLaunch: false,
  stopClientOnExit: false,
  uiStyle: 'win11',
  locale: locale.value,
})

const newSubnet = ref('')
const autostart = ref(false)
const configDir = ref('')
const error = ref('')

// ── 即时保存 ────────────────────────────────────────────────────────
//
// 设置页没有「保存」按钮：改完就落盘。500ms 内的连续改动合并成一次写盘，
// 免得手填路径时每敲一个字符写一次配置文件。
//
// 只有两种情况不落盘——「选了来源却没选目录」和「MTU 越界」：
// 存进去就是一份自相矛盾的配置，下次连接报的错会莫名其妙。
// 这时底部状态栏直接说明原因，改对了自动接着存。

type AutoState = 'idle' | 'saving' | 'saved' | 'blocked' | 'error'

const autoState = ref<AutoState>('idle')
const autoError = ref('')
const blockReason = ref('')

let saveTimer: number | undefined
let flashTimer: number | undefined
/** 已落盘那份配置的指纹，用来判断「这次改动是不是真的变了」 */
let savedFingerprint = ''
/** 已落盘的客户端目录，用来决定保存后要不要重扫/重校验 */
let savedDir: string | null = null
/** 自己写回 settings 时置位，避免回填把正在编辑的 draft 冲掉 */
let echoGuard = false

// ── 客户端目录的三种来源 ────────────────────────────────────────────
//
// 这一块的核心是「选了什么」和「实际能用什么」必须都看得见。
// 用户选了一个后来被卸载掉的目录时，界面要当场说明，
// 而不是等他点"连接"再收到一句莫名其妙的报错。

const candidates = ref<ClientCandidate[]>([])
const scanning = ref(false)
const scanError = ref('')
/** 用户手填路径的即时校验结果；null 表示没验过 */
const customCheck = ref<ClientCandidate | null>(null)
const customChecking = ref(false)
/** 「自己指定」被展开时，手填框要自动聚焦 */
const customInput = ref<HTMLInputElement | null>(null)

const SOURCE_OPTIONS = computed<{ value: ClientSource; title: string; note: string }[]>(() => [
  { value: 'bundled', title: t('client.bundledTitle'), note: t('client.bundledNote') },
  { value: 'detected', title: t('client.detectedTitle'), note: t('client.detectedNote') },
  { value: 'custom', title: t('client.customTitle'), note: t('client.customNote') },
])

/**
 * 可选界面风格。
 *
 * 卡片结构完全一致，只有预览图配色不同（由 .preview.<value> 决定），
 * 所以统一成一份数据驱动；新增一套风格只需要在这里加一行 + 一个 CSS 块。
 */
const STYLE_OPTIONS = computed<{ value: UiStyle; name: string; desc: string }[]>(() => [
  { value: 'win11', name: t('settings.style1'), desc: t('settings.style1Desc') },
  { value: 'deepin', name: t('settings.style2'), desc: t('settings.style2Desc') },
  { value: 'mac', name: t('settings.style3'), desc: t('settings.style3Desc') },
])

/** 内置客户端总在第一位；它不来自扫盘，所以单独取 */
const bundled = computed(() => candidates.value.find((item) => item.source === 'bundled') ?? null)

/** 扫到的目录（去掉与内置重复的） */
const detected = computed(() =>
  candidates.value.filter((item) => item.source === 'detected' && item.dir !== bundled.value?.dir),
)

const detectedOptions = computed<SelectOption<string>[]>(() => {
  if (!detected.value.length) {
    return [{ label: t('client.noOthersOption'), value: '', hint: t('client.noOthersHint'), disabled: true }]
  }
  return detected.value.map((item) => ({
    // 不可用的也列出来：用户可能就是想知道"为什么没识别到我那个"
    label: item.usable ? basename(item.dir) : `${basename(item.dir)}${t('client.unusableSuffix')}`,
    value: item.dir,
    hint: `${item.dir}${item.version ? ` · v${item.version}` : ''}${item.problem ? ` · ${item.problem}` : ''}`,
    disabled: !item.usable,
    tag: item.dir === draft.vpnDir ? t('client.currentInUse') : undefined,
  }))
})

/** 当前来源下实际生效的目录，用于底部结论行 */
const effective = computed(() => {
  if (draft.clientSource === 'bundled') return bundled.value
  const dir = draft.vpnDir?.trim()
  if (!dir) return null
  return candidates.value.find((item) => item.dir === dir) ?? customCheck.value
})

/** 结论行：一句话说清"现在到底用哪个目录、能不能用" */
const verdict = computed(() => {
  const source = draft.clientSource ?? 'bundled'
  if (source === 'bundled') {
    const b = bundled.value
    if (!b) return { tone: 'bad' as const, text: t('client.bundledMissing') }
    if (!b.usable) return { tone: 'bad' as const, text: t('client.bundledIncomplete', { problem: b.problem ?? '' }) }
    return { tone: 'ok' as const, text: t('client.useBundled', { version: b.version ?? '?', dir: b.dir }) }
  }
  const dir = draft.vpnDir?.trim()
  if (!dir) {
    return { tone: 'warn' as const, text: source === 'custom' ? t('client.fillPath') : t('client.chooseDir') }
  }
  const target = effective.value
  if (!target) return { tone: 'warn' as const, text: t('client.checkingDir') }
  if (!target.usable) {
    return { tone: 'bad' as const, text: t('client.dirUnusable', { problem: target.problem ?? t('client.filesIncomplete') }) }
  }
  if (!target.writable) {
    return { tone: 'warn' as const, text: t('client.readonlyDir', { dir: target.dir }) }
  }
  return { tone: 'ok' as const, text: t('client.usingDir', { dir: target.dir, version: target.version ?? '?' }) }
})

function basename(dir: string): string {
  const parts = dir.split('/').filter(Boolean)
  return parts[parts.length - 1] ?? dir
}

async function scanClients(quiet = false) {
  scanning.value = true
  scanError.value = ''
  try {
    candidates.value = await invoke<ClientCandidate[]>('list_client_candidates')
    if (!quiet) {
      const usable = candidates.value.filter((item) => item.usable).length
      showToast(usable > 1 ? t('client.scanFound', { n: usable }) : t('client.scanDone'))
    }
  } catch (err) {
    scanError.value = String(err)
  } finally {
    scanning.value = false
  }
}

/**
 * 切到「自己指定」时，把当前选中的目录带过去当起点。
 * 用户往往只是想微调路径，从空白开始会让他把整条路径重打一遍。
 */
function pickSource(source: ClientSource) {
  draft.clientSource = source
  customCheck.value = null
  if (source === 'custom' && !draft.vpnDir) draft.vpnDir = ''
  if (source === 'custom') nextTick(() => customInput.value?.focus())
}

async function checkCustomDir() {
  const dir = draft.vpnDir?.trim()
  if (!dir) return
  customChecking.value = true
  try {
    customCheck.value = await invoke<ClientCandidate>('validate_client_dir', { dir })
  } catch (err) {
    error.value = String(err)
    customCheck.value = null
  } finally {
    customChecking.value = false
  }
}

/**
 * 目录选择器。
 *
 * Tauri 没有内置文件对话框插件（要另装 tauri-plugin-dialog），
 * 为了一个选目录的功能引入新原生依赖不划算，所以这里直接输入路径。
 * 好处是可以粘贴路径，坏处是打错字——所以「校验」按钮必须存在，
 * 且校验不过要给出具体缺哪个文件。
 */
watch(
  () => draft.vpnDir,
  () => {
    // 路径一改，上一次的校验结果就过期了，不清掉会误导用户
    customCheck.value = null
  },
)

const accountOptions = computed<SelectOption<string | null>[]>(() => [
  { label: t('settings.noAccount'), value: null },
  ...(overview.value?.accounts ?? []).map((account) => ({ label: account.name, value: account.name })),
])

function fill(source: Settings) {
  Object.assign(draft, {
    ...source,
    routes: { ...source.routes, subnets: [...source.routes.subnets] },
  })
}

/**
 * 发出去的那份配置。
 *
 * 「内置客户端」不看路径，所以这里强制把 vpnDir 归零：既不会把上次
 * 选过的目录写进配置，也保证下面算指纹时两次可比较。
 * draft 里的 vpnDir 仍然留着，切回「搜索到的」时还能选回原来那个。
 */
function snapshot(source: Settings = draft): Settings {
  const out: Settings = {
    ...source,
    routes: { ...source.routes, subnets: [...source.routes.subnets] },
  }
  if ((out.clientSource ?? 'bundled') === 'bundled') out.vpnDir = null
  return out
}

/**
 * 配置指纹：只回答「和已落盘的那份相比变没变」。
 *
 * 键名必须排序后再序列化——Rust 侧序列化的字段顺序和这里不一样，
 * 不排序的话同一份配置会算出两个指纹，导致每次进页面都白写一次磁盘。
 */
function fingerprint(source: Settings): string {
  return JSON.stringify(snapshot(source), (_key, value) =>
    value && typeof value === 'object' && !Array.isArray(value)
      ? Object.fromEntries(Object.keys(value).sort().map((key) => [key, value[key]]))
      : value,
  )
}

/** 这份 draft 能不能落盘；返回不能保存的原因，null 表示可以 */
function blockedBecause(): string | null {
  const source = draft.clientSource ?? 'bundled'
  if (source !== 'bundled' && !draft.vpnDir?.trim()) {
    return source === 'custom' ? t('client.fillPath') : t('client.chooseDir')
  }
  const mtu = Number(draft.routes.mtu)
  if (!Number.isFinite(mtu) || mtu < 576 || mtu > 1500) return t('settings.mtuRange')
  return null
}

/**
 * 落盘。所有改动最终都收到这里，包括离开页面时那一次补写。
 *
 * 失败不回滚 draft：用户填的东西不能因为一次写盘失败就消失，
 * 状态栏会挂出后端给的原文，点「重试保存」再来一次。
 */
async function persist() {
  window.clearTimeout(saveTimer)
  saveTimer = undefined

  const payload = snapshot()
  if (fingerprint(payload) === savedFingerprint) return

  // 上一次写盘还没回来：排到队尾再来一次，别让两个请求抢同一个配置文件
  if (autoState.value === 'saving') {
    queueSave()
    return
  }

  const blocked = blockedBecause()
  if (blocked) {
    blockReason.value = blocked
    autoState.value = 'blocked'
    return
  }

  blockReason.value = ''
  autoState.value = 'saving'
  autoError.value = ''
  try {
    const saved = await invoke<Settings>('save_settings', { settings: payload })
    savedFingerprint = fingerprint(payload)
    echoGuard = true
    settings.value = saved
    autoState.value = 'saved'
    window.clearTimeout(flashTimer)
    flashTimer = window.setTimeout(() => {
      if (autoState.value === 'saved') autoState.value = 'idle'
    }, 1800)

    // 目录换了才重扫/重校验：其余开关不值得为它们再跑一轮扫描
    const dir = saved.vpnDir ?? null
    if (dir !== savedDir) {
      savedDir = dir
      void scanClients(true)
      if ((saved.clientSource ?? 'bundled') === 'custom' && dir) void checkCustomDir()
    }
  } catch (err) {
    autoState.value = 'error'
    autoError.value = String(err)
  }
}

/** 改动排一次保存；连续改动只写最后那一次 */
function queueSave() {
  window.clearTimeout(saveTimer)
  saveTimer = window.setTimeout(() => void persist(), 500)
}

const autoText = computed(() => {
  switch (autoState.value) {
    case 'saving':
      return t('common.saving')
    case 'saved':
      return t('settings.saved')
    case 'blocked':
      return t('settings.pendingHint', { reason: blockReason.value })
    case 'error':
      return autoError.value
    default:
      return t('settings.autoSaveHint')
  }
})

onMounted(async () => {
  await loadSettings()
  if (settings.value) fill(settings.value)
  // 基准值要在回填之后再取，否则第一次改动会和空指纹比较
  if (settings.value) {
    savedFingerprint = fingerprint(settings.value)
    savedDir = settings.value.vpnDir ?? null
  }
  autostart.value = await invoke<boolean>('autostart_enabled')
  configDir.value = await invoke<string>('config_dir_path')
  // 扫盘放在最后：它最慢（几百毫秒），不能让上面的内容跟着等
  void scanClients(true)
})

watch(settings, (value) => {
  if (!value) return
  // 刚由本页写下去的那一份不回填：回填会清掉 draft 里记住的目录，
  // 也会把用户正在输入的路径打断
  if (echoGuard) {
    echoGuard = false
    return
  }
  fill(value)
  savedFingerprint = fingerprint(value)
})

function pickStyle(style: UiStyle) {
  draft.uiStyle = style
  // watch 只在取值变化时触发；点已选中的卡片也要能恢复预览
  applyUiStyle(style)
}

function pickLocale(value: Locale) {
  draft.locale = value
  applyLocale(value)
}

// 风格切换即时预览整套界面，不用等保存
watch(
  () => draft.uiStyle,
  (value) => applyUiStyle(value),
)

// 语言切换同样即时生效
watch(
  () => draft.locale,
  (value) => applyLocale(value),
)

// 页面上任何一处改动都排队保存；这一层是"不用点保存"的全部实现
watch(draft, queueSave, { deep: true })

// 切走页面时还在 500ms 窗口里的改动要立刻写掉，不能丢
onBeforeUnmount(() => {
  window.clearTimeout(flashTimer)
  if (saveTimer !== undefined) {
    window.clearTimeout(saveTimer)
    saveTimer = undefined
    void persist()
  }
})

function addSubnet() {
  const value = newSubnet.value.trim()
  if (!value) return
  if (!/^\d{1,3}(\.\d{1,3}){3}\/\d{1,2}$/.test(value)) {
    error.value = t('settings.subnetError', { value })
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
    showToast(autostart.value ? t('settings.autostartOn') : t('settings.autostartOff'))
  } catch (err) {
    error.value = String(err)
    autostart.value = !autostart.value
  }
}

</script>

<template>
  <section class="settings">
    <div class="settings-scroll">
    <header>
      <h1>{{ t('settings.title') }}</h1>
      <p class="hint">{{ t('settings.subtitle', { dir: configDir }) }}</p>
    </header>

    <div class="card block">
      <h2>{{ t('settings.language') }}</h2>
      <div class="langs">
        <button
          type="button"
          class="lang-card"
          :class="{ on: draft.locale === 'zh' }"
          @click="pickLocale('zh')"
        >
          <span class="radio" :class="{ on: draft.locale === 'zh' }">
            <Check v-if="draft.locale === 'zh'" :size="11" />
          </span>
          {{ t('settings.languageZh') }}
        </button>
        <button
          type="button"
          class="lang-card"
          :class="{ on: draft.locale === 'en' }"
          @click="pickLocale('en')"
        >
          <span class="radio" :class="{ on: draft.locale === 'en' }">
            <Check v-if="draft.locale === 'en'" :size="11" />
          </span>
          {{ t('settings.languageEn') }}
        </button>
      </div>
      <p class="hint" style="margin: 2px 0 0">{{ t('settings.languageHint') }}</p>
    </div>

    <div class="card block">
      <h2>{{ t('settings.style') }}</h2>
      <div class="styles">
        <button
          v-for="option in STYLE_OPTIONS"
          :key="option.value"
          type="button"
          class="style-card"
          :class="{ on: draft.uiStyle === option.value }"
          @click="pickStyle(option.value)"
        >
          <span class="preview" :class="option.value">
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
          <span class="style-name">{{ option.name }}</span>
          <span class="hint">{{ option.desc }}</span>
        </button>
      </div>
      <p class="hint" style="margin: 2px 0 0">{{ t('settings.stylePreview') }}</p>
    </div>

    <div class="card block">
      <h2>{{ t('settings.defaults') }}</h2>
      <div class="row wrap">
        <div class="field" style="min-width: 200px">
          <label>{{ t('settings.activeAccount') }}</label>
          <SelectInput v-model="draft.activeAccount" :options="accountOptions" />
        </div>
        <label class="switch">
          <input v-model="draft.routes.autoConnect" type="checkbox" />
          <span>{{ t('settings.autoConnect') }}</span>
        </label>
        <label class="switch">
          <input v-model="draft.startMinimized" type="checkbox" />
          <span>{{ t('settings.startMinimized') }}</span>
        </label>
        <label class="switch">
          <input v-model="draft.notifications" type="checkbox" />
          <span>{{ t('settings.notifications') }}</span>
        </label>
      </div>
      <div class="row" style="margin-top: 10px">
        <label class="switch">
          <input v-model="autostart" type="checkbox" @change="toggleAutostart" />
          <span>{{ t('settings.autostart') }}</span>
        </label>
      </div>
      <div class="row" style="margin-top: 10px; gap: 18px">
        <label class="switch">
          <input v-model="draft.startClientOnLaunch" type="checkbox" />
          <span>{{ t('settings.startClient') }}</span>
        </label>
        <label class="switch">
          <input v-model="draft.stopClientOnExit" type="checkbox" />
          <span>{{ t('settings.stopClient') }}</span>
        </label>
      </div>
      <p class="hint" style="margin: 8px 0 0">
        {{ t('settings.clientHint') }}
      </p>
    </div>

    <div class="card block">
      <div class="row-between">
        <div>
          <h2>{{ t('settings.routesTitle') }}</h2>
          <p class="hint">{{ t('settings.routesIntro') }}</p>
        </div>
        <button class="ghost" @click="resetRoutes"><RotateCcw :size="14" /> {{ t('settings.restoreDefaults') }}</button>
      </div>

      <div class="subnets">
        <span v-for="subnet in draft.routes.subnets" :key="subnet" class="chip mono">
          {{ subnet }}
          <button @click="removeSubnet(subnet)" :aria-label="t('common.delete')">×</button>
        </span>
        <span v-if="!draft.routes.subnets.length" class="hint">{{ t('settings.noSubnets') }}</span>
      </div>
      <div class="row" style="margin-top: 8px">
        <input v-model="newSubnet" type="text" :placeholder="t('settings.subnetPlaceholder')" style="max-width: 240px" @keyup.enter="addSubnet" />
        <button class="ghost" @click="addSubnet"><Plus :size="14" /> {{ t('settings.add') }}</button>
      </div>

      <div class="row wrap" style="margin-top: 12px">
        <div class="field" style="width: 140px">
          <label>{{ t('settings.mtu') }}</label>
          <NumberInput v-model="draft.routes.mtu" :min="576" :max="1500" :step="10" />
        </div>
        <label class="switch">
          <input v-model="draft.routes.dhcp" type="checkbox" />
          <span>{{ t('settings.flushAddr') }}</span>
        </label>
      </div>
      <p class="hint" style="margin: 8px 0 0">
        {{ t('settings.mtuHint') }}
      </p>
      <p class="hint" style="margin: 6px 0 0">
        {{ t('settings.dhcpHintPrefix') }}<span class="mono">dhclient -1</span>{{ t('settings.dhcpHintMid') }}
      </p>
    </div>

    <div class="card block">
      <div class="row-between">
        <div>
          <h2>{{ t('client.title') }}</h2>
          <p class="hint" style="margin: 4px 0 0">
            {{ t('client.introBefore') }}<span class="mono">vpnclient</span>{{ t('client.introAfter') }}
          </p>
        </div>
        <button class="ghost" :disabled="scanning" @click="scanClients()">
          <RefreshCw :size="13" :class="{ spin: scanning }" /> {{ scanning ? t('client.scanning') : t('client.rescan') }}
        </button>
      </div>

      <div class="sources">
        <button
          v-for="option in SOURCE_OPTIONS"
          :key="option.value"
          type="button"
          class="source-card"
          :class="{ on: (draft.clientSource ?? 'bundled') === option.value }"
          @click="pickSource(option.value)"
        >
          <span class="source-head">
            <span class="radio" :class="{ on: (draft.clientSource ?? 'bundled') === option.value }">
              <Check v-if="(draft.clientSource ?? 'bundled') === option.value" :size="11" />
            </span>
            <span class="source-title">{{ option.title }}</span>
            <!-- 内置的可用性直接标在标题旁：选它之前就该知道它好不好 -->
            <span v-if="option.value === 'bundled' && bundled" class="badge" :class="bundled.usable ? 'ok' : 'bad'">
              {{ bundled.usable ? `v${bundled.version ?? '?'}` : t('client.missing') }}
            </span>
            <span v-if="option.value === 'detected'" class="badge off">
              {{ t('client.foundCount', { n: detected.filter((item) => item.usable).length }) }}
            </span>
          </span>
          <span class="source-note">{{ option.note }}</span>
        </button>
      </div>

      <!-- 内置：无需选择具体目录，只把路径摆出来 -->
      <div v-if="(draft.clientSource ?? 'bundled') === 'bundled'" class="source-body">
        <p class="hint" style="margin: 0">
          {{ t('client.pathLabel') }} <span class="mono">{{ bundled?.dir ?? '/usr/lib/softether-tray/client' }}</span>
          <template v-if="bundled?.version"> · {{ t('client.versionLabel') }} <span class="mono">{{ bundled.version }}</span></template>
        </p>
        <p class="hint" style="margin: 4px 0 0">
          {{ t('client.bundledBody') }}
        </p>
      </div>

      <!-- 搜索到的：下拉挑一个 -->
      <div v-else-if="draft.clientSource === 'detected'" class="source-body">
        <div class="row wrap" style="align-items: flex-end">
          <div class="field" style="flex: 1; min-width: 260px">
            <label>{{ t('client.installedClients') }}</label>
            <SelectInput v-model="draft.vpnDir" :options="detectedOptions" :placeholder="t('client.choosePlaceholder')" />
          </div>
        </div>
        <p v-if="scanning" class="hint" style="margin: 6px 0 0">{{ t('client.scanningLocations') }}</p>
        <p v-else-if="scanError" class="error" style="margin: 6px 0 0">{{ scanError }}</p>
        <p v-else-if="!detected.length" class="hint" style="margin: 6px 0 0">
          {{ t('client.noOthersBody') }}
        </p>
      </div>

      <!-- 自己指定：手填 + 校验 -->
      <div v-else class="source-body">
        <div class="row wrap" style="align-items: flex-end">
          <div class="field" style="flex: 1; min-width: 260px">
            <label>{{ t('client.customDirLabel') }}</label>
            <input
              ref="customInput"
              v-model="draft.vpnDir"
              type="text"
              placeholder="/usr/local/vpnclient"
              @keyup.enter="checkCustomDir"
            />
          </div>
          <button :disabled="customChecking || !draft.vpnDir?.trim()" @click="checkCustomDir">
            <ShieldCheck :size="13" /> {{ customChecking ? t('client.validating') : t('client.validate') }}
          </button>
        </div>

        <div v-if="customCheck" class="check" :class="customCheck.usable ? 'ok' : 'bad'">
          <!-- 直接分支而不是 <component :is="'Check'">：传字符串只在
               全局注册时才解析得到，这些图标是局部导入的组件对象。 -->
          <Check v-if="customCheck.usable" :size="14" />
          <TriangleAlert v-else :size="14" />
          <span v-if="customCheck.usable">
            {{ t('client.usableVersion', { version: customCheck.version ?? t('client.unknownVersion') }) }}
            <template v-if="!customCheck.writable">{{ t('client.readonlyInline') }}</template>
          </span>
          <span v-else>{{ customCheck.problem ?? t('client.notAClient') }}</span>
        </div>

        <p class="hint" style="margin: 6px 0 0">
          {{ t('client.requiredBefore') }}<span class="mono">vpnclient</span>{{ t('client.requiredSep1') }}<span class="mono">vpncmd</span>{{ t('client.requiredSep2') }}<span class="mono">hamcore.se2</span>{{ t('client.requiredAfter') }}
        </p>
      </div>

      <!-- 结论行：不管选了哪个来源，都在这里给一句话 -->
      <div class="verdict" :class="verdict.tone">
        <span class="dot" />
        <span>{{ verdict.text }}</span>
      </div>
    </div>

    <div class="card block">
      <h2>{{ t('client.diagTitle') }}</h2>
      <div class="field" style="max-width: 320px">
        <label>{{ t('client.diagTargetLabel') }}</label>
        <input v-model="draft.diagTarget" type="text" :placeholder="t('diag.targetPlaceholder')" />
      </div>
      <p class="hint" style="margin: 4px 0 0">{{ t('client.diagHint') }}</p>
    </div>

    <p v-if="error" class="error">{{ error }}</p>
    </div>

    <div class="foot">
      <span class="auto-status" :class="autoState">
        <i class="dot" />
        {{ autoText }}
      </span>
      <!-- 写盘失败时给一个明确的重试入口：改别处也能重试，但那太隐蔽了 -->
      <button v-if="autoState === 'error'" class="ghost" @click="persist">
        <RefreshCw :size="13" /> {{ t('settings.retrySave') }}
      </button>
    </div>
  </section>
</template>

<style scoped>
.settings {
  display: flex;
  flex-direction: column;
  height: 100%;
  min-height: 0;
}

.settings-scroll {
  flex: 1;
  min-height: 0;
  padding: 18px 20px 24px;
  display: flex;
  flex-direction: column;
  overflow: auto;
  gap: 12px;
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

/* 状态栏独立于滚动区，始终固定在设置页底部。 */
.foot {
  flex: none;
  display: flex;
  justify-content: space-between;
  align-items: center;
  gap: 12px;
  padding: 8px 20px 10px;
  background: var(--bg);
  border-top: 0.5px solid var(--line);
}

.auto-status {
  display: inline-flex;
  align-items: center;
  gap: 6px;
  font-size: 12px;
  color: var(--text-hint);
}

.auto-status.saving {
  color: var(--text-soft);
}

.auto-status.saving .dot {
  animation: status-pulse 1s ease-in-out infinite;
}

.auto-status.saved {
  color: var(--ok);
}

.auto-status.blocked {
  color: var(--warn);
}

.auto-status.error {
  color: var(--danger);
}

@keyframes status-pulse {
  50% {
    opacity: 0.2;
  }
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

.langs {
  display: flex;
  flex-wrap: wrap;
  gap: 8px;
}

.lang-card {
  display: inline-flex;
  align-items: center;
  gap: 7px;
  min-width: 132px;
  padding: 7px 12px;
  background: var(--panel);
  border: 1px solid var(--line-strong);
  border-radius: var(--radius-ctl);
  color: var(--text);
  text-align: left;
}

.lang-card:hover {
  border-color: var(--accent);
}

.lang-card.on {
  border-color: var(--accent);
  background: var(--accent-soft);
  color: var(--accent-ink);
}

/* 三张风格卡片等宽均分；窄窗口按 auto-fit 折成两行，不再靠固定宽度硬撑 */
.styles {
  display: flex;
  flex-wrap: wrap;
  gap: 12px;
}

.style-card {
  flex: 1 1 196px;
  display: flex;
  flex-direction: column;
  align-items: flex-start;
  gap: 3px;
  padding: 10px;
  background: var(--panel);
  border: 1px solid var(--line-strong);
  border-radius: var(--radius);
  text-align: left;
  /* button 全局是 nowrap；这里带说明文字，必须允许换行，否则会溢出卡片 */
  white-space: normal;
}

/* 说明文字撑满卡片宽度并允许任意位置断行，避免长中文/英文溢出边框 */
.style-card .hint {
  align-self: stretch;
  line-height: 1.45;
  overflow-wrap: anywhere;
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

/* 缩略示意图刻意不跟主题变量走：每套预览各画各的风格，切换前后都得是“目标风格的样子” */
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

/* macOS：侧栏压深一档、选中是蓝色小圆角、卡片只有一层极淡描影。
   与 Deepin 预览的差别刻意做在「侧栏底色 + 选中项形状」上，
   否则两套都蓝都白，缩略图根本分不出来。 */
.preview.mac {
  background: #f5f5f7;
}

.preview.mac .pv-side {
  position: relative;
  /* 顶上留出一条给交通灯，否则会压在第一个导航条上 */
  padding-top: 17px;
  background: #eceef1;
  border-right: 1px solid #e3e3e8;
}

/* 缩略图里最认得出的就是这三颗圆点，比配色差异有效得多 */
.preview.mac .pv-side::before {
  content: "";
  position: absolute;
  left: 7px;
  top: 7px;
  width: 5px;
  height: 5px;
  border-radius: 50%;
  background: #ff5f57;
  box-shadow: 7px 0 0 #febc2e, 14px 0 0 #28c840;
}

.preview.mac .pv-nav {
  background: #dfdfe4;
  border-radius: 4px;
}

.preview.mac .pv-nav.on {
  background: #007aff;
}

.preview.mac .pv-card {
  background: #ffffff;
  border-radius: 8px;
  box-shadow: 0 1px 3px rgba(0, 0, 0, 0.09);
}

.preview.mac .pv-btn {
  background: #007aff;
  border-radius: 6px;
}

/* ── 客户端来源选择 ─────────────────────────────────────────────── */

/* 三张卡片横排，窄窗口自动折行；min-width 保证折行后仍是完整卡片 */
.sources {
  display: flex;
  flex-wrap: wrap;
  gap: 8px;
}

.source-card {
  flex: 1 1 200px;
  display: flex;
  flex-direction: column;
  gap: 4px;
  padding: 10px 12px;
  text-align: left;
  background: var(--panel);
  border: 1px solid var(--line-strong);
  border-radius: var(--radius-ctl);
  cursor: pointer;
  transition: border-color 0.12s ease, background 0.12s ease;
  /* 同样覆盖 button 的 nowrap，说明文字要在卡片内换行 */
  white-space: normal;
}

.source-card:hover {
  border-color: var(--text-hint);
}

/* 选中态靠强调色描边 + 浅底，不用阴影：这套设计语言里阴影只给浮层 */
.source-card.on {
  border-color: var(--accent);
  background: var(--accent-soft);
}

.source-head {
  display: flex;
  align-items: center;
  gap: 7px;
  flex-wrap: wrap;
}

.source-title {
  font-size: 13px;
  font-weight: 550;
  color: var(--text);
}

.source-card.on .source-title {
  color: var(--accent-ink);
}

.radio {
  flex: none;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 15px;
  height: 15px;
  border-radius: 50%;
  border: 1.5px solid var(--line-strong);
  color: #fff;
  transition: border-color 0.12s ease, background 0.12s ease;
}

.radio.on {
  background: var(--accent);
  border-color: var(--accent);
}

.source-note {
  font-size: 11.5px;
  color: var(--text-hint);
  line-height: 1.45;
}

/* 选中卡片下的具体配置区：左侧一道竖线表示"属于上面这张卡" */
.source-body {
  padding: 2px 0 0 10px;
  margin-left: 1px;
  border-left: 2px solid var(--accent-line);
  display: flex;
  flex-direction: column;
  gap: 2px;
}

/* 校验结果 */
.check {
  display: flex;
  align-items: center;
  gap: 7px;
  margin-top: 8px;
  padding: 7px 10px;
  border-radius: var(--radius-ctl);
  font-size: 12px;
  line-height: 1.5;
}

.check.ok {
  background: var(--ok-soft);
  color: var(--ok);
  border: 0.5px solid var(--ok-line);
}

.check.bad {
  background: var(--danger-soft);
  color: var(--danger);
  border: 0.5px solid var(--danger-line);
}

/* 结论行：这是用户唯一必须看到的一句话，所以放在卡片最底部、加底色 */
.verdict {
  display: flex;
  align-items: center;
  gap: 8px;
  margin-top: 2px;
  padding: 8px 12px;
  border-radius: var(--radius-ctl);
  font-size: 12.5px;
  line-height: 1.5;
  background: var(--panel-muted);
  border: 0.5px solid var(--line);
  color: var(--text-soft);
}

.verdict.ok {
  background: var(--ok-soft);
  border-color: var(--ok-line);
  color: var(--ok);
}

.verdict.warn {
  background: var(--warn-soft);
  border-color: var(--warn-line);
  color: var(--warn);
}

.verdict.bad {
  background: var(--danger-soft);
  border-color: var(--danger-line);
  color: var(--danger);
}

.verdict .dot {
  background: currentColor;
}

/* 扫描图标转起来：不加这个动画用户会以为"没反应" */
.spin {
  animation: spin 0.9s linear infinite;
}

@keyframes spin {
  to {
    transform: rotate(360deg);
  }
}
</style>
