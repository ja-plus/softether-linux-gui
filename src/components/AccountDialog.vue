<script setup lang="ts">
import { computed, onMounted, reactive, ref } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import { Eye, EyeOff, Plus } from 'lucide-vue-next'
import { Modal as AModal } from '@arco-design/web-vue'
import '@arco-design/web-vue/es/modal/style/css.js'
import SelectInput from './SelectInput.vue'
import NumberInput from './NumberInput.vue'
import InfoTip from './InfoTip.vue'
import {
  busy,
  overview,
  run,
  showToast,
  type AccountDetail,
  type NicInfo,
  type Overview,
} from '../lib/api'
import { t } from '../lib/i18n'

/**
 * 连接配置表单弹窗（Arco Modal 外壳）。
 *
 * 一般不在模板里挂它，走 lib/dialog.ts 的 openAccountDialog() 函数式调用；
 * 等关窗动画播完才由 close 带出"是否保存成功"，宿主据此卸载。
 */
const props = defineProps<{ name: string | null }>()
const emit = defineEmits<{ close: [saved: boolean] }>()

const form = reactive({
  name: '',
  host: '',
  port: 12443,
  hub: '',
  user: '',
  nic: '',
  password: '',
  encrypted: true,
  // 默认勾选「不使用 UDP 加速」：公司网络封 UDP，启用它会掉线/高流量
  // （tet 配置当年就是这么解决的）。保存时应用会把它写入客户端配置文件。
  noUdpAccel: true,
  compress: false,
  maxTcp: 1,
})

const loading = ref(!!props.name)
const newNic = ref('')
const saving = ref(false)
/** 密码是否以明文显示（眼睛按钮切换） */
const showPassword = ref(false)
// 打开弹窗时该项的原值：保存后有变化才提示"重启客户端生效"
const initialNoUdp = ref(false)
/** 保存成功过才在关窗时报告 true */
const saved = ref(false)
/** Modal 的开关要自己拿着：传死 true 的话它内部 close() 关不掉 */
const visible = ref(true)

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
      initialNoUdp.value = detail.noUdpAccel
      form.compress = detail.compress
      form.maxTcp = detail.maxTcp || 1
    } catch (err) {
      // 读不到详情时表单是空的，用 toast 说清楚原因，别让用户对着空表单发呆
      showToast(String(err), true)
    }
  }
  form.nic = form.nic || overview.value?.nics[0]?.name || ''
  loading.value = false
})

/** Modal 确定按钮的前置钩子：返回 false 保持弹窗打开（保存失败时重试） */
async function save(): Promise<boolean> {
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
    // UDP 加速项变了会重启客户端服务才能生效，单说"已保存"会让用户以为改完了
    if (form.noUdpAccel !== initialNoUdp.value) {
      showToast(t('account.udpApplied'))
    }
    saved.value = true
    return true
  } catch (err) {
    // 保存失败用顶部 toast：对话框保持打开、输入原样保留，
    // 用户改一下就能重试。内联红条会把对话框底部撑开，按钮往下跳。
    showToast(String(err), true)
    return false
  } finally {
    saving.value = false
  }
}

async function addNic() {
  const name = newNic.value.trim()
  if (!name) return
  const result = await run('nic', () => invoke<NicInfo[]>('create_nic', { name }))
  if (result !== null) {
    // 顺手把新的网卡列表写回总状态：设置页的适配器卡片读的就是它，
    // 不写回会出现"这里建好了，那边列表还是空的"
    if (overview.value) overview.value = { ...overview.value, nics: result }
    form.nic = name
    newNic.value = ''
    showToast(t('account.nicCreated'))
  }
}
</script>

<template>
  <AModal
    v-model:visible="visible"
    :title="title"
    :width="560"
    title-align="start"
    :mask-closable="false"
    :ok-text="saving ? t('common.saving') : t('common.save')"
    :cancel-text="t('common.cancel')"
    :ok-button-props="{ disabled: !canSave || saving || loading }"
    :on-before-ok="save"
    @close="emit('close', saved)"
  >
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
        <div class="password">
          <input
            v-model="form.password"
            :type="showPassword ? 'text' : 'password'"
            autocomplete="off"
            :placeholder="t('account.passwordPlaceholder')"
          />
          <button
            type="button"
            class="peek"
            :aria-label="showPassword ? t('common.hidePassword') : t('common.showPassword')"
            :title="showPassword ? t('common.hidePassword') : t('common.showPassword')"
            @mousedown.prevent
            @click="showPassword = !showPassword"
          >
            <EyeOff v-if="showPassword" :size="15" />
            <Eye v-else :size="15" />
          </button>
        </div>
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
        <label class="switch">
          <input v-model="form.noUdpAccel" type="checkbox" />
          <span>{{ t('account.noUdpAccel') }}</span>
          <InfoTip :text="t('account.udpTip')" />
        </label>
        <div class="field" style="max-width: 150px">
          <label>{{ t('account.maxTcp') }}</label>
          <NumberInput v-model="form.maxTcp" :min="1" :max="32" />
        </div>
      </div>

    </div>
  </AModal>
</template>

<style scoped>
.form {
  display: flex;
  flex-direction: column;
  gap: 12px;
}

.two {
  display: flex;
  gap: 10px;
}

.two:first-child > .field:first-child {
  flex: 1 1 auto;
}

.two:first-child > .field:last-child {
  flex: 0 0 120px;
}

.two + .two {
  display: flex;
}

.two + .two > .field {
  flex: 1 1 0;
}

/*
 * 密码框右端长一个眼睛按钮：边框与聚焦环画在容器上、input 自己无边框，
 * 和 NumberInput 一样避免两层圆角打架。
 */
.password {
  display: flex;
  align-items: stretch;
  width: 100%;
  min-height: 34px;
  background: var(--panel);
  border: 0.5px solid var(--line-strong);
  border-radius: var(--radius-ctl);
  overflow: hidden;
  transition: border-color 0.12s ease, box-shadow 0.12s ease;
}

.password:hover {
  border-color: var(--text-hint);
}

.password:focus-within {
  border-color: var(--accent);
  box-shadow: 0 0 0 3px var(--focus-ring);
}

.password input {
  flex: 1;
  min-width: 0;
  /* 全局 input 带 min-height:34px，清零才不把容器顶成 36px */
  min-height: 0;
  padding: 6px 10px;
  border: none;
  border-radius: 0;
  background: transparent;
}

/* 描边和环已由容器的 :focus-within 表达 */
.password input:focus {
  border-color: transparent;
  box-shadow: none;
}

.peek {
  flex: none;
  display: flex;
  align-items: center;
  justify-content: center;
  width: 32px;
  padding: 0;
  border: none;
  background: transparent;
  color: var(--text-soft);
  cursor: pointer;
  transition: background 0.1s ease, color 0.1s ease;
}

.peek:hover,
.peek:focus-visible {
  background: var(--accent-soft);
  color: var(--accent);
}

.switch {
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

/* 禁用项（如「不使用 UDP 加速」）：文字压暗，鼠标给出不可改的信号 */
.switch input:disabled + span {
  color: var(--text-hint);
  cursor: not-allowed;
}
</style>
