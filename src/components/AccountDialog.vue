<script setup lang="ts">
import { computed, onMounted, reactive, ref } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import { X, Plus, Check } from 'lucide-vue-next'
import SelectInput from './SelectInput.vue'
import { busy, showToast, overview, run, type AccountDetail, type Overview } from '../lib/api'

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
const title = computed(() => (isEdit.value ? `编辑 ${props.name}` : '新建连接配置'))
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
    showToast(isEdit.value ? '配置已保存' : '配置已创建')
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
    showToast('虚拟网络适配器已创建')
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

      <p v-if="loading" class="hint">正在读取配置…</p>

      <div v-else class="form">
        <div class="two">
          <div class="field">
            <label>配置名称</label>
            <input v-model="form.name" type="text" placeholder="例如 tet" />
          </div>
          <div class="field">
            <label>端口</label>
            <input v-model.number="form.port" type="number" min="1" max="65535" />
          </div>
        </div>

        <div class="field">
          <label>服务器地址</label>
          <input v-model="form.host" type="text" placeholder="例如 hzvpn.example.cn" />
          <span class="hint">SoftEther VPN Server 的域名或 IP，不带端口。</span>
        </div>

        <div class="two">
          <div class="field">
            <label>虚拟 HUB</label>
            <input v-model="form.hub" type="text" placeholder="例如 hubname" />
          </div>
          <div class="field">
            <label>用户名</label>
            <input v-model="form.user" type="text" placeholder="登录 VPN 的用户名" />
          </div>
        </div>

        <div class="field">
          <label>密码 {{ isEdit ? '（留空表示保持不变）' : '' }}</label>
          <input v-model="form.password" type="password" autocomplete="off" placeholder="只在保存时使用，不会写入配置" />
        </div>

        <div class="field">
          <label>虚拟网络适配器</label>
          <div class="row">
            <SelectInput
              v-model="form.nic"
              :options="nicOptions"
              placeholder="（还没有适配器）"
              style="flex: 1; min-width: 140px"
            />
            <input v-model="newNic" type="text" placeholder="新适配器名" style="max-width: 140px" />
            <button class="ghost" :disabled="!!busy" @click="addNic"><Plus :size="14" /> 创建</button>
          </div>
        </div>

        <div class="switches">
          <label class="switch">
            <input v-model="form.encrypted" type="checkbox" />
            <span>启用 TLS 加密</span>
          </label>
          <label class="switch">
            <input v-model="form.compress" type="checkbox" />
            <span>启用压缩</span>
          </label>
          <div class="field" style="max-width: 150px">
            <label>TCP 连接数</label>
            <input v-model.number="form.maxTcp" type="number" min="1" max="32" />
          </div>
        </div>

        <div class="note">
          <strong>UDP 加速：Linux 客户端没有开关</strong>
          <p class="hint">
            官方 4.44 Linux 客户端不支持 <span class="mono">AccountDetailSet /NOUDPACCEL</span>。本机已有的
            <span class="mono">tet</span> 配置是通过客户端配置文件的
            <span class="mono">NoUdpAcceleration</span> 关掉的；新建配置默认带上 UDP 加速，如果遇到频繁掉线或异常高流量，
            先停止客户端再改该文件。
          </p>
        </div>

        <p v-if="error" class="error">{{ error }}</p>
      </div>

      <div class="foot">
        <button class="ghost" @click="emit('close')">取消</button>
        <button class="primary" :disabled="!canSave || saving || loading" @click="save">
          <Check :size="14" /> {{ saving ? '保存中…' : '保存' }}
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
  min-height: 32px;
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
