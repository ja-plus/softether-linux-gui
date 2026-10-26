<script setup lang="ts">
import { computed, onMounted, reactive, ref } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import { X, Plus, Check } from 'lucide-vue-next'
import SelectInput from './SelectInput.vue'
import NumberInput from './NumberInput.vue'
import { busy, showToast, overview, run, type AccountDetail, type Overview } from '../lib/api'
import { t } from '../lib/i18n'

const props = defineProps<{ name: string | null }>()
const emit = defineEmits<{ close: [] }>()

const form = reactive({
  name: '',
  host: '',
  port: 12443,
  hub: '',
  user: '',
  nic: '',
  password: '',
  encrypted: true,
  noUdpAccel: true,
  compress: false,
  maxTcp: 1,
})

const loading = ref(!!props.name)
const error = ref('')
const newNic = ref('')
const saving = ref(false)

const isEdit = computed(() => !!props.name)
const title = computed(() => (isEdit.value ? t('account.editTitle', { name: props.name }) : t('account.newTitle')))
const canSave = computed(
  () => !!form.name.trim() && !!form.host.trim() && !!form.hub.trim() && !!form.user.trim() && form.port > 0,
)
const nicOptions = computed(() =>
  (overview.value?.nics ?? []).map((nic) => ({ label: nic.name, value: nic.name })),
)

onMounted(async () => {
  if (props.name) {
    try {
      const detail = await invoke<AccountDetail>('account_detail', { name: props.name })
      form.name = detail.name
      form.host = detail.host
      form.port = detail.port || 443
      form.hub = detail.hub
      form.user = detail.user
      form.nic = detail.nic || overview.value?.nics[0]?.name || ''
      form.encrypted = detail.encrypted
      form.noUdpAccel = detail.noUdpAccel
      form.compress = detail.compress
      form.maxTcp = detail.maxTcp || 1
    } catch (err) {
      error.value = String(err)
    }
  }
  form.nic = form.nic || overview.value?.nics[0]?.name || ''
  loading.value = false
})

async function save() {
  error.value = ''
  saving.value = true
  try {
    // save_account 的返回值就是保存后的完整状态，直接写回，不再多读一遍 overview
    overview.value = await invoke<Overview>('save_account', {
      original: props.name,
      input: {
        name: form.name.trim(),
        host: form.host.trim(),
        port: Number(form.port),
        hub: form.hub.trim(),
        user: form.user.trim(),
        nic: form.nic.trim(),
        password: form.password,
        encrypted: form.encrypted,
        noUdpAccel: form.noUdpAccel,
        compress: form.compress,
        maxTcp: Number(form.maxTcp),
      },
    })
    showToast(isEdit.value ? t('account.saved') : t('account.created'))
    emit('close')
  } catch (err) {
    error.value = String(err)
  } finally {
    saving.value = false
  }
}

async function addNic() {
  const name = newNic.value.trim()
  if (!name) return
  const result = await run('nic', () => invoke('create_nic', { name }))
  if (result !== null) {
    form.nic = name
    newNic.value = ''
    showToast(t('account.nicCreated'))
  }
}
</script>

<template>
  <div class="overlay" @click.self="emit('close')">
    <div class="dialog">
      <header>
        <h2>{{ title }}</h2>
        <button class="icon" @click="emit('close')"><X :size="16" /></button>
      </header>

      <p v-if="loading" class="hint">{{ t('account.loading') }}</p>

      <div v-else class="form">
        <div class="two">
          <div class="field">
            <label>{{ t('account.name') }}</label>
            <input v-model="form.name" type="text" :placeholder="t('account.namePlaceholder')" />
          </div>
          <div class="field">
            <label>{{ t('account.port') }}</label>
            <NumberInput v-model="form.port" :min="1" :max="65535" :step="1" />
          </div>
        </div>

        <div class="field">
          <label>{{ t('account.host') }}</label>
          <input v-model="form.host" type="text" :placeholder="t('account.hostPlaceholder')" />
          <span class="hint">{{ t('account.hostHint') }}</span>
        </div>

        <div class="two">
          <div class="field">
            <label>{{ t('account.hub') }}</label>
            <input v-model="form.hub" type="text" :placeholder="t('account.hubPlaceholder')" />
          </div>
          <div class="field">
            <label>{{ t('account.user') }}</label>
            <input v-model="form.user" type="text" :placeholder="t('account.userPlaceholder')" />
          </div>
        </div>

        <div class="field">
          <label>{{ t('account.password') }}{{ isEdit ? t('account.passwordKeep') : '' }}</label>
          <input v-model="form.password" type="password" autocomplete="off" :placeholder="t('account.passwordPlaceholder')" />
        </div>

        <div class="field">
          <label>{{ t('account.nic') }}</label>
          <div class="row">
            <SelectInput
              v-model="form.nic"
              :options="nicOptions"
              :placeholder="t('account.noNic')"
              style="flex: 1; min-width: 140px"
            />
            <input v-model="newNic" type="text" :placeholder="t('account.newNicPlaceholder')" style="max-width: 140px" />
            <button class="ghost" :disabled="!!busy" @click="addNic"><Plus :size="14" /> {{ t('account.createNic') }}</button>
          </div>
        </div>

        <div class="switches">
          <label class="switch">
            <input v-model="form.encrypted" type="checkbox" />
            <span>{{ t('account.tls') }}</span>
          </label>
          <label class="switch">
            <input v-model="form.compress" type="checkbox" />
            <span>{{ t('account.compress') }}</span>
          </label>
          <div class="field" style="max-width: 150px">
            <label>{{ t('account.maxTcp') }}</label>
            <NumberInput v-model="form.maxTcp" :min="1" :max="32" />
          </div>
        </div>

        <div class="note">
          <strong>{{ t('account.udpTitle') }}</strong>
          <p class="hint">
            {{ t('account.udpBefore') }}<span class="mono">AccountDetailSet /NOUDPACCEL</span>{{ t('account.udpMid1') }}<span class="mono">tet</span>{{ t('account.udpMid2') }}<span class="mono">NoUdpAcceleration</span>{{ t('account.udpAfter') }}
          </p>
        </div>

        <p v-if="error" class="error">{{ error }}</p>
      </div>

      <div class="foot">
        <button class="ghost" @click="emit('close')">{{ t('common.cancel') }}</button>
        <button class="primary" :disabled="!canSave || saving || loading" @click="save">
          <Check :size="14" /> {{ saving ? t('common.saving') : t('common.save') }}
        </button>
      </div>
    </div>
  </div>
</template>

<style scoped>
.dialog {
  max-width: 560px;
}

.icon {
  background: none;
  border: none;
  color: var(--text-soft);
  padding: 4px;
}

.form {
  display: flex;
  flex-direction: column;
  gap: 12px;
}

.two {
  display: grid;
  grid-template-columns: 1fr 120px;
  gap: 10px;
}

.two + .two {
  grid-template-columns: 1fr 1fr;
}

.switches {
  display: flex;
  align-items: flex-end;
  gap: 18px;
  flex-wrap: wrap;
}

.switch {
  display: inline-flex;
  align-items: center;
  gap: 6px;
  font-size: 13px;
  color: var(--text-soft);
  min-height: 34px;
}

.note {
  background: var(--panel-muted);
  border: 0.5px solid var(--line);
  border-radius: var(--radius-ctl);
  padding: 10px 12px;
}

.note p {
  margin: 4px 0 0;
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

.foot {
  display: flex;
  justify-content: flex-end;
  gap: 8px;
  margin-top: 16px;
}

.foot button {
  display: inline-flex;
  align-items: center;
  gap: 6px;
}
</style>
