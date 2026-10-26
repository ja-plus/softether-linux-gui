<script setup lang="ts">
import { ref } from 'vue'
import { Plus, RotateCcw } from 'lucide-vue-next'
import NumberInput from '../NumberInput.vue'
import { t } from '../../lib/i18n'
import { DEFAULT_MTU, DEFAULT_SUBNETS, MTU_RANGE, useSettingsDraft } from '../../composables/useSettingsDraft'

const draft = useSettingsDraft()

const newSubnet = ref('')
const error = ref('')

/** 只收 IPv4 CIDR：IPv6 走隧道要靠路由表另行配置，这里不假装支持 */
const CIDR = /^\d{1,3}(\.\d{1,3}){3}\/\d{1,2}$/

function addSubnet() {
  const value = newSubnet.value.trim()
  if (!value) return
  if (!CIDR.test(value)) {
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

/** 「恢复默认」把三段私网 + MTU 一起还原；dhcp 也归位，避免只还原一半 */
function resetRoutes() {
  draft.routes.subnets = [...DEFAULT_SUBNETS]
  draft.routes.mtu = DEFAULT_MTU
  draft.routes.dhcp = false
}
</script>

<template>
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
      <input
        v-model="newSubnet"
        type="text"
        :placeholder="t('settings.subnetPlaceholder')"
        style="max-width: 240px"
        @keyup.enter="addSubnet"
      />
      <button class="ghost" @click="addSubnet"><Plus :size="14" /> {{ t('settings.add') }}</button>
    </div>
    <p v-if="error" class="error">{{ error }}</p>

    <div class="row wrap" style="margin-top: 12px">
      <div class="field" style="width: 140px">
        <label>{{ t('settings.mtu') }}</label>
        <NumberInput v-model="draft.routes.mtu" :min="MTU_RANGE.min" :max="MTU_RANGE.max" :step="10" />
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
</template>

<style scoped>
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
</style>
