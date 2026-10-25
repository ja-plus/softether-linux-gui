<script setup lang="ts">
import { computed, onMounted, ref, watch } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import { Activity, Play } from 'lucide-vue-next'
import { formatBytes, overview, settings, showToast, type DiagResult } from '../lib/api'

const target = ref('')
const count = ref(20)
const running = ref(false)
const results = ref<DiagResult[]>([])
const error = ref('')

const iface = computed(() => overview.value?.iface ?? null)

onMounted(() => {
  target.value = settings.value?.diagTarget || ''
})

// settings 是异步加载的，挂载时可能还是 null；到达后要回填默认目标，否则按钮永远 disabled
watch(
  () => settings.value?.diagTarget,
  (value) => {
    if (!target.value.trim() && value) target.value = value
  },
)

async function runCheck() {
  error.value = ''
  running.value = true
  try {
    const result = await invoke<DiagResult>('run_diagnostics', {
      target: target.value.trim(),
      count: Number(count.value) || 20,
    })
    results.value = [result, ...results.value].slice(0, 8)
    showToast(`${result.target} · 丢包 ${result.lossPercent.toFixed(0)}%`)
  } catch (err) {
    error.value = String(err)
  } finally {
    running.value = false
  }
}

function quality(result: DiagResult): { text: string; tone: string } {
  if (result.received === 0) return { text: '不可达', tone: 'bad' }
  if (result.lossPercent >= 5) return { text: '丢包严重', tone: 'bad' }
  if (result.lossPercent > 0) return { text: '有丢包', tone: 'warn' }
  if ((result.rttAvgMs ?? 0) > 120) return { text: '延迟偏高', tone: 'warn' }
  return { text: '正常', tone: 'ok' }
}
</script>

<template>
  <section class="diag">
    <header class="head">
      <div>
        <h1>网络诊断</h1>
        <p class="hint">
          通过系统 ping 检查隧道质量。丢包和延迟都只代表 ICMP，不等于 TCP 业务的体感；但它能快速区分“隧道坏了”和“服务端慢”。
        </p>
      </div>
    </header>

    <div class="card panel">
      <div class="row">
        <div class="field" style="flex: 1">
          <label>目标地址</label>
          <input v-model="target" type="text" placeholder="例如 10.10.4.160" />
        </div>
        <div class="field" style="width: 110px">
          <label>次数</label>
          <input v-model.number="count" type="number" min="1" max="100" />
        </div>
        <button class="primary" :disabled="running || !target.trim()" @click="runCheck">
          <Play :size="14" /> {{ running ? '检测中…' : '开始检测' }}
        </button>
      </div>
      <p class="hint" style="margin: 10px 0 0">
        {{ iface ? `当前经 ${iface}（${overview?.ifaceIpv4 ?? '无 IP'}，MTU ${overview?.ifaceMtu ?? '—'}）` : '当前没有 VPN 隧道，检测的是本地网络' }}
      </p>
      <p v-if="error" class="error">{{ error }}</p>
    </div>

    <div class="card results">
      <header class="list-head">
        <h2>最近结果</h2>
        <span class="hint">{{ results.length }} 条</span>
      </header>
      <div v-if="!results.length" class="empty">
        <Activity :size="20" class="empty-icon" />
        <p class="muted">还没有检测结果。输入公司内网地址开始。</p>
      </div>
      <table v-else>
        <thead>
          <tr>
            <th>目标</th>
            <th>质量</th>
            <th class="num">丢包</th>
            <th class="num">平均</th>
            <th class="num">最小 / 最大</th>
            <th class="num">收 / 发</th>
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
      <h2>怎么看这些数</h2>
      <ul class="hint">
        <li>ICMP 0% 丢包但网页很慢 → 问题在 TCP 建连或服务端，不在带宽。</li>
        <li>ICMP 稳定丢包且延迟抖动 → 先在本地网关复测，再测 VPN 服务器外层地址，最后测内网目标。</li>
        <li>大包失败、小包正常 → MTU 不匹配，降低隧道 MTU 再测一次。</li>
        <li>累计流量 {{ formatBytes(overview?.snapshot?.rxBytes ?? 0) }} 下载 / {{ formatBytes(overview?.snapshot?.txBytes ?? 0) }} 上传。</li>
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
  height: 100%;
  overflow: auto;
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
