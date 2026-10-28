import { inject, provide, reactive, type InjectionKey } from 'vue'
import type { Settings } from '../lib/api'
import { locale } from '../lib/i18n'

/**
 * 设置类页面的共享草稿。
 *
 * 每张卡片都改同一份 `Settings`，逐层 v-model 传下去会变成一堆
 * `update:xxx` 事件，所以这里用 provide/inject 共享同一个 reactive 对象：
 * 卡片只管改自己的字段，落盘交给 useAutoSave 统一处理。
 *
 * 目前有两个提供方：`SettingsTab`（语言/样式/默认/路由/诊断五张卡）和
 * `ClientTab`（客户端目录）。provide 必须在**卡片的直接父组件**里做。
 */

/** 默认网段：RFC1918 三段全下，绝大多数内网都落在这三个里 */
export const DEFAULT_SUBNETS = ['10.0.0.0/8', '172.16.0.0/12', '192.168.0.0/16']

/** 隧道 MTU 默认值 */
export const DEFAULT_MTU = 1400

/** MTU 合法区间；前后端校验共用一份，别各写各的 */
export const MTU_RANGE = { min: 576, max: 1500 } as const

export function createSettingsDraft(): Settings {
  return reactive<Settings>({
    vpnDir: null,
    clientSource: 'bundled',
    vpncmdPath: null,
    activeAccount: null,
    notifications: true,
    startMinimized: true,
    diagTarget: '',
    routes: { subnets: [...DEFAULT_SUBNETS], autoConnect: false, dhcp: true, mtu: DEFAULT_MTU },
    startClientOnLaunch: false,
    stopClientOnExit: false,
    uiStyle: 'win11',
    locale: locale.value,
  })
}

const DRAFT_KEY: InjectionKey<Settings> = Symbol('settings-draft')

export function provideSettingsDraft(draft: Settings) {
  provide(DRAFT_KEY, draft)
}

/** 取当前页面的草稿；没有 provide 就直接报错，免得拿到 undefined 静默失效 */
export function useSettingsDraft(): Settings {
  const draft = inject(DRAFT_KEY)
  if (!draft) throw new Error('useSettingsDraft() 只能在 SettingsTab / ClientTab 的子组件里调用')
  return draft
}
