import { computed, onBeforeUnmount, ref, watch } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import { settings, type Settings } from '../lib/api'
import { t, type MessageKey } from '../lib/i18n'
import { MTU_RANGE } from './useSettingsDraft'

/**
 * 设置页的「改完就落盘」。
 *
 * 没有保存按钮：500ms 内的连续改动合并成一次写盘，免得手填路径时
 * 每敲一个字符写一次配置文件。
 *
 * 只有两种情况不落盘——「选了来源却没选目录」和「MTU 越界」：
 * 存进去就是一份自相矛盾的配置，下次连接报的错会莫名其妙。
 * 这时底部状态栏直接说明原因，改对了自动接着存。
 */

export type AutoState = 'idle' | 'saving' | 'saved' | 'blocked' | 'error'

/** 连续改动合并窗口 */
const SETTLE_MS = 500
/** 「已保存」提示停留时长，之后回落到 idle 的常驻说明 */
const FLASH_MS = 1800

/**
 * `idleHint` 是底部状态栏空闲时那句话的词条 key。
 *
 * 设置页和「SoftEther 客户端」页共用这套自动保存，但两页要说的事不一样
 * （一页讲路由要重连才生效，一页讲换目录会重启客户端），所以由调用方指定。
 */
export function useAutoSave(
  draft: Settings,
  idleHint: MessageKey = 'settings.autoSaveHint',
) {
  const autoState = ref<AutoState>('idle')
  const autoError = ref('')
  const blockReason = ref('')

  let saveTimer: number | undefined
  let flashTimer: number | undefined
  /** 已落盘那份配置的指纹，用来判断「这次改动是不是真的变了」 */
  let savedFingerprint = ''
  /** 自己写回 settings 时置位，避免回填把正在编辑的 draft 冲掉 */
  let echoGuard = false

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
    if (!Number.isFinite(mtu) || mtu < MTU_RANGE.min || mtu > MTU_RANGE.max) return t('settings.mtuRange')
    return null
  }

  /**
   * 没什么要存的：把「被卡住」和「写盘失败」这两种状态收掉，底部回到常驻说明。
   *
   * 不能无条件调用——`saved` 的绿字要让 flashTimer 自己闪完，
   * `saving` 更不能在这里改写，那是另一个请求的结果。
   */
  function settle() {
    blockReason.value = ''
    autoError.value = ''
    autoState.value = 'idle'
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
    // 草稿已经等于磁盘上那一份，没有东西要写。
    //
    // 但这里不能只是 return：典型场景是「自己指定」填了空路径被 blocked，
    // 用户点回「内置客户端」后 snapshot 会把 vpnDir 归零，指纹正好等于
    // 已落盘那份——于是走这条路，状态永远停在「尚未保存：请填写目录路径」，
    // 而界面上明明已经是内置客户端、结论行还是绿的。
    if (fingerprint(payload) === savedFingerprint) {
      if (autoState.value === 'blocked' || autoState.value === 'error') settle()
      return
    }

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
      // 写盘期间草稿又被改了（比如连点两下来源卡片）：这次响应落的是旧值，
      // 立刻再排一次，否则状态栏显示「已保存」而磁盘上还是上一版。
      if (fingerprint(draft) !== savedFingerprint) queueSave()
      window.clearTimeout(flashTimer)
      flashTimer = window.setTimeout(() => {
        if (autoState.value === 'saved') autoState.value = 'idle'
      }, FLASH_MS)
    } catch (err) {
      autoState.value = 'error'
      autoError.value = String(err)
    }
  }

  /** 改动排一次保存；连续改动只写最后那一次 */
  function queueSave() {
    window.clearTimeout(saveTimer)
    saveTimer = window.setTimeout(() => void persist(), SETTLE_MS)
  }

  /** 把一份配置铺进 draft；子网数组必须深拷贝，否则改草稿会改到 settings 上 */
  function fill(source: Settings) {
    Object.assign(draft, {
      ...source,
      routes: { ...source.routes, subnets: [...source.routes.subnets] },
    })
  }

  /**
   * 记下「当前 draft 就是磁盘上那份」。
   *
   * 必须在 fill() 之后调用，否则第一次改动会和一个空指纹比较，
   * 进页面什么都没改也会白写一次。
   */
  function markBaseline(source: Settings = draft) {
    savedFingerprint = fingerprint(source)
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
        return t(idleHint)
    }
  })

  // 外部改了配置（比如另一个入口写了盘）就回填，但自己写下去的那一份不回填：
  // 回填会清掉 draft 里记住的目录，也会把用户正在输入的路径打断。
  watch(settings, (value) => {
    if (!value) return
    if (echoGuard) {
      echoGuard = false
      return
    }
    fill(value)
    markBaseline(value)
  })

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

  return { autoState, autoError, autoText, persist, queueSave, fill, markBaseline }
}
