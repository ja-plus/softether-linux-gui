<script setup lang="ts">
import { computed, onMounted, ref, watch } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import { Activity, Play } from 'lucide-vue-next'
import { formatBytes, overview, settings, showToast, type DiagResult } from '../lib/api'
import NumberInput from './NumberInput.vue'
import InfoTip from './InfoTip.vue'
import { t } from '../lib/i18n'

const target = ref('')
const autoTarget = ref('')
const count = ref(20)
const running = ref(false)
const results = ref<DiagResult[]>([])
const error = ref('')

const iface = computed(() => overview.value?.iface ?? null)

/**
 * 空输入时真正会 ping 的地址：设置里填过的优先，否则用后端按当前隧道算出来的网关。
 *
 * 这里刻意不预置任何内网地址——每个人的内网都不一样，写死一个只会让诊断打到
 * 一个与他无关的地址上。
 */
const effectiveTarget = computed(() => target.value.trim() || autoTarget.value)

onMounted(async () => {
  target.value = settings.value?.diagTarget || ''
  await refreshAutoTarget()
})

// settings 是异步加载的，挂载时可能还是 null；到达后要回填默认目标，否则按钮永远 disabled
watch(
  () => settings.value?.diagTarget,
  (value) => {
    if (!target.value.trim() && value) target.value = value
  },
)

// 隧道换了（连上 / 断开 / 换网卡）默认地址就变了，跟着重算
watch(() => overview.value?.iface, refreshAutoTarget)

async function refreshAutoTarget() {
  try {
    autoTarget.value = (await invoke<string | null>('diag_default_target')) ?? ''
  } catch {
    autoTarget.value = ''
  }
}

async function runCheck() {
  error.value = ''
  running.value = true
  try {
    const result = await invoke<DiagResult>('run_diagnostics', {
      target: effectiveTarget.value,
      count: Number(count.value) || 20,
    })
    results.value = [result, ...results.value].slice(0, 8)
    showToast(t('diag.done', { target: result.target, loss: result.lossPercent.toFixed(0) }))
  } catch (err) {
    error.value = String(err)
  } finally {
    running.value = false
  }
}

function quality(result: DiagResult): { text: string; tone: string } {
  if (result.received === 0) return { text: t('diag.unreachable'), tone: 'bad' }
  if (result.lossPercent >= 5) return { text: t('diag.severeLoss'), tone: 'bad' }
  if (result.lossPercent > 0) return { text: t('diag.lossy'), tone: 'warn' }
  if ((result.rttAvgMs ?? 0) > 120) return { text: t('diag.highLatency'), tone: 'warn' }
  return { text: t('diag.normal'), tone: 'ok' }
}
</script>

<template>
  <section class="diag">
    <header class="head">
      <div class="title-line">
        <h1>{{ t('diag.title') }}</h1>
        <InfoTip :text="t('diag.subtitle')" />
      </div>
    </header>

    <div class="card panel">
      <div class="row">
        <div class="field" style="flex: 1">
          <label>{{ t('diag.target') }}</label>
          <input
            v-model="target"
            type="text"
            :placeholder="autoTarget ? t('diag.autoTarget', { target: autoTarget }) : t('diag.targetPlaceholder')"
          />
        </div>
        <div class="field" style="width: 110px">
          <label>{{ t('diag.count') }}</label>
          <NumberInput v-model="count" :min="1" :max="100" />
        </div>
        <button class="primary" :disabled="running || !effectiveTarget" @click="runCheck">
          <Play :size="14" /> {{ running ? t('diag.running') : t('diag.start') }}
        </button>
      </div>
      <p class="hint" style="margin: 10px 0 0">
        {{ iface
          ? t('diag.currentVia', { iface, ip: overview?.ifaceIpv4 ?? t('common.noIp'), mtu: overview?.ifaceMtu ?? '—' })
          : t('diag.noTunnel') }}
      </p>
      <p v-if="error" class="error">{{ error }}</p>
    </div>

    <div class="card results">
      <header class="list-head">
        <h2>{{ t('diag.recent') }}</h2>
        <span class="hint">{{ t('diag.countLabel', { n: results.length }) }}</span>
      </header>
      <div v-if="!results.length" class="empty">
        <Activity :size="20" class="empty-icon" />
        <p class="muted">{{ t('diag.empty') }}</p>
      </div>
      <table v-else>
        <thead>
          <tr>
            <th>{{ t('diag.targetCol') }}</th>
            <th>{{ t('diag.qualityCol') }}</th>
            <th class="num">{{ t('diag.lossCol') }}</th>
            <th class="num">{{ t('diag.avgCol') }}</th>
            <th class="num">{{ t('diag.minmaxCol') }}</th>
            <th class="num">{{ t('diag.packetsCol') }}</th>
          </tr>
        </thead>
        <tbody>
          <tr v-for="item in results" :key="`${item.target}-${item.sent}-${item.received}-${item.rttAvgMs}`">
            <td class="mono">{{ item.target }}</td>
            <td><span class="badge" :class="quality(item).tone">{{ quality(item).text }}</span></td>
            <td class="num">{{ item.lossPercent.toFixed(0) }}%</td>
            <td class="num">{{ item.rttAvgMs === null ? '—' : `${item.rttAvgMs.toFixed(1)} ms` }}</td>
            <td class="num">
              {{ item.rttMinMs === null || item.rttMaxMs === null ? '—' : `${item.rttMinMs.toFixed(1)} / ${item.rttMaxMs.toFixed(1)}` }}
            </td>
            <td class="num">{{ item.received }} / {{ item.sent }}</td>
          </tr>
        </tbody>
      </table>
    </div>

    <div class="card tips">
      <h2>{{ t('diag.tipsTitle') }}</h2>
      <ul class="hint">
        <li>{{ t('diag.tip1') }}</li>
        <li>{{ t('diag.tip2') }}</li>
        <li>{{ t('diag.tip3') }}</li>
        <li>{{ t('diag.tip4', { rx: formatBytes(overview?.snapshot?.rxBytes ?? 0), tx: formatBytes(overview?.snapshot?.txBytes ?? 0) }) }}</li>
      </ul>
    </div>
  </section>
</template>

<style scoped>
.diag {
  padding: 18px 20px 24px;
  display: flex;
  flex-direction: column;
  gap: 12px;
  flex: 1 1 auto;
  min-height: 0;
  overflow-y: auto;
}

.title-line {
  display: flex;
  align-items: center;
  gap: 8px;
}

.panel {
  padding: 14px 16px;
}

.panel .row {
  align-items: flex-end;
}

button.primary {
  display: inline-flex;
  align-items: center;
  gap: 6px;
  margin-bottom: 1px;
}

.error {
  background: var(--danger-soft);
  color: var(--danger);
  border: 0.5px solid var(--danger-line);
  border-radius: var(--radius-ctl);
  padding: 8px 10px;
  margin: 10px 0 0;
  font-size: 12px;
}

.results {
  padding: 0;
  overflow: hidden;
}

.list-head {
  display: flex;
  justify-content: space-between;
  align-items: baseline;
  padding: 12px 16px;
  border-bottom: 0.5px solid var(--line);
}

.empty {
  padding: 28px 16px;
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: 6px;
}

.empty-icon {
  color: var(--text-hint);
}

table {
  width: 100%;
  border-collapse: collapse;
}

th,
td {
  text-align: left;
  padding: 8px 16px;
  border-bottom: 0.5px solid var(--line);
  font-size: 12px;
}

th {
  color: var(--text-soft);
  font-weight: 400;
  background: var(--panel-muted);
}

tbody tr:last-child td {
  border-bottom: none;
}

.num {
  text-align: right;
}

.tips {
  padding: 14px 16px 6px;
}

.tips ul {
  margin: 6px 0 8px;
  padding-left: 18px;
}

.tips li {
  margin-bottom: 3px;
}
</style>
