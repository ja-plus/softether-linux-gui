<script setup lang="ts">
import { computed, nextTick, onMounted, ref, watch } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import { Check, Loader2, Power, RefreshCw, ShieldCheck, TriangleAlert } from 'lucide-vue-next'
import { confirmDialog } from '../../lib/dialog'
import SelectInput, { type SelectOption } from '../SelectInput.vue'
import {
  busy,
  overview,
  showToast,
  settings,
  startService,
  stopService,
  switchClientDir,
  type ClientCandidate,
  type ClientSource,
} from '../../lib/api'
import { t } from '../../lib/i18n'
import { useSettingsDraft } from '../../composables/useSettingsDraft'

/**
 * 客户端目录的三种来源。
 *
 * 这一块的核心是「选了什么」和「实际能用什么」必须都看得见。
 * 用户选了一个后来被卸载掉的目录时，界面要当场说明，
 * 而不是等他点"连接"再收到一句莫名其妙的报错。
 */

const draft = useSettingsDraft()

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

/** 客户端服务是否在跑。启停按钮跟着它切换红/绿 */
const serviceRunning = computed(() => overview.value?.serviceRunning ?? false)
/** 启停服务进行中：按钮转圈，避免连点 */
const serviceBusy = computed(() => busy.value === 'service')

function snapshotDraft() {
  return {
    source: (draft.clientSource ?? 'bundled') as ClientSource,
    dir: draft.vpnDir ?? null,
  }
}

/**
 * 需要切换时拦一下，返回 true 表示"已弹确认，调用方别再动草稿"。
 *
 * 只有服务在跑才算切换；服务停着的时候改目录本来就是安全的，
 * 直接写设置、下次启动自然生效。
 *
 * 客户端运行中改目录=切换（要停服务、会断隧道），所以**先弹确认再改草稿**。
 * 记住改之前的值，用户取消时原样恢复——否则草稿已经变了、自动保存又把它写盘，
 * 就会出现"设置显示新目录、实际跑旧客户端"的错乱。
 */
function requestSwitch(dir: string | null | undefined, source: ClientSource): boolean {
  const target = dir?.trim()
  if (!target || !serviceRunning.value) return false
  if (target === overview.value?.runningDir) return false
  const previous = snapshotDraft()
  void confirmDialog({
    title: t('overview.switchTitle'),
    body: t('overview.switchBody'),
    confirmText: t('overview.switchConfirm'),
    danger: true,
  }).then(async (ok) => {
    if (!ok) {
      draft.clientSource = previous.source
      if (previous.dir !== null) draft.vpnDir = previous.dir
      return
    }
    const result = await switchClientDir(target, source)
    if (result) {
      // 后端按切换后的来源写了配置，草稿跟上，免得下一次自动保存把它改回旧值
      draft.clientSource = source
      draft.vpnDir = target
      customCheck.value = null
    }
  })
  return true
}

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
    // 手填的目录不在扫描结果里，界面就会一直停在"正在检查"：
    // 扫完顺手验一次，让结论行当场给出答案。
    void verifyCustomIfNeeded()
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
  // 「内置」的目标目录是确定的：点它就是切换，运行中先弹确认
  if (source === 'bundled' && requestSwitch(bundled.value?.dir, 'bundled')) return
  // 「搜索到的」/「自己指定」还要选具体目录，等选定时再弹（见 onPickDetected / checkCustomDir）
  if (source !== 'bundled' && requestSwitch(draft.vpnDir, source)) return
  draft.clientSource = source
  customCheck.value = null
  if (source === 'custom' && !draft.vpnDir) draft.vpnDir = ''
  if (source === 'custom') nextTick(() => customInput.value?.focus())
}

/** 「搜索到的」下拉里选定具体目录 = 一次切换（清空选择不算） */
function onPickDetected(dir: string | null) {
  if (!dir) return
  draft.vpnDir = dir
  if (requestSwitch(dir, 'detected')) return
}

async function checkCustomDir() {
  const dir = draft.vpnDir?.trim()
  if (!dir) return
  customChecking.value = true
  try {
    customCheck.value = await invoke<ClientCandidate>('validate_client_dir', { dir })
  } catch (err) {
    scanError.value = String(err)
    customCheck.value = null
  } finally {
    customChecking.value = false
  }
  // 校验通过就等于"我要用这个目录"，运行中此时问一句要不要立即切换
  if (customCheck.value?.usable) requestSwitch(dir, 'custom')
}

/** 只在「自己指定 + 有路径 + 扫描结果里没有这个路径」时才自动校验 */
function verifyCustomIfNeeded() {
  if ((draft.clientSource ?? 'bundled') !== 'custom') return
  const dir = draft.vpnDir?.trim()
  if (!dir || customCheck.value || customChecking.value) return
  if (candidates.value.some((item) => item.dir === dir)) return
  void checkCustomDir()
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

// 切到「自己指定」时若已有路径，直接验一次，不用等用户点按钮
watch(
  () => draft.clientSource,
  (source) => {
    if (source === 'custom') verifyCustomIfNeeded()
  },
)

// 配置从磁盘回填的时机和扫盘完成的时机不固定，两边都试一次：
// 谁在最后拿到数据，谁就能把结论行定下来，不会停在"正在检查"。
watch(
  settings,
  (value) => {
    if (value) verifyCustomIfNeeded()
  },
  { immediate: true },
)

onMounted(() => {
  // 扫描最慢（几百毫秒），交给它自己的 loading 状态，不阻塞别的卡片
  void scanClients(true)
})
</script>

<template>
  <div class="card block">
    <div class="row-between">
      <div>
        <h2>{{ t('client.title') }}</h2>
        <p class="hint" style="margin: 4px 0 0">
          {{ t('client.introBefore') }}<span class="mono">vpnclient</span>{{ t('client.introAfter') }}
        </p>
      </div>
      <div class="head-actions">
        <!-- 启停服务放在客户端卡片上：它管的就是这个客户端，
             放在连接页反而让人以为跟某条隧道绑定。 -->
        <button
          v-if="serviceRunning"
          class="danger"
          :disabled="!!busy"
          :title="t('overview.stopTooltip')"
          @click="stopService()"
        >
          <Loader2 v-if="serviceBusy" :size="14" class="spin" />
          <Power v-else :size="14" />
          {{ t('overview.stopService') }}
        </button>
        <button
          v-else
          class="success"
          :disabled="!!busy"
          :title="t('overview.startTooltip')"
          @click="startService()"
        >
          <Loader2 v-if="serviceBusy" :size="14" class="spin" />
          <Power v-else :size="14" />
          {{ t('overview.startService') }}
        </button>
        <!-- 重新扫描只留图标：文字在这张卡片上抢不过启停按钮 -->
        <button
          class="ghost icon-only"
          :disabled="scanning"
          :aria-label="t('client.rescan')"
          :title="scanning ? t('client.scanning') : t('client.rescan')"
          @click="scanClients()"
        >
          <RefreshCw :size="15" :class="{ spin: scanning }" />
        </button>
      </div>
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
            {{ bundled.usable ? `v${bundled.version ?? '?'}` : bundled.writable ? t('client.missing') : t('client.readOnly') }}
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
          <SelectInput
            v-model="draft.vpnDir"
            :options="detectedOptions"
            :placeholder="t('client.choosePlaceholder')"
            @update:model-value="onPickDetected"
          />
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
</template>

<style scoped>
/* 头部右侧：启停按钮在前，扫描图标在后，图标按钮不吃多余宽度 */
.head-actions {
  display: flex;
  align-items: center;
  gap: 8px;
  flex: none;
}

.head-actions button {
  display: inline-flex;
  align-items: center;
  gap: 6px;
}

/* 三张来源卡片用响应式网格均分；窄窗口自动减少列数 */
.sources {
  display: grid;
  grid-template-columns: repeat(auto-fit, minmax(min(200px, 100%), 1fr));
  gap: 8px;
}

.source-card {
  min-width: 0;
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
