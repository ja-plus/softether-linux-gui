import { createVNode, render, type Component } from 'vue'
import AccountDialog from '../components/AccountDialog.vue'
import ConfirmDialog from '../components/ConfirmDialog.vue'

/**
 * 弹窗的函数式调用入口。
 *
 * 以前弹窗靠模板里的 `v-if` + 事件回调开关，每个调用点都要养一个
 * "待确认"状态（pendingXxx），问完还得手动清。这里统一改成：
 *
 *   const ok = await confirmDialog({ ... })
 *   const saved = await openAccountDialog(name)
 *
 * 弹窗本体保持独立组件（ConfirmDialog / AccountDialog，外壳都是 Arco Modal），
 * 挂载和卸载由本模块代劳：组件在关窗动画播完后发 `close` 带上结果，
 * 这里才卸载并兑现 Promise——提前卸载会把退场动画掐掉。
 */

export interface ConfirmOptions {
  title: string
  body: string
  /** 确认按钮文案；不传就用「确定」 */
  confirmText?: string
  /** true 时确认按钮用危险色：会中断当前动作的操作都该是红的 */
  danger?: boolean
}

/** 通用二次确认。点确认 → true；取消/点遮罩/ESC/关闭 → false */
export function confirmDialog(options: ConfirmOptions): Promise<boolean> {
  return mountDialog(ConfirmDialog, { ...options })
}

/** 账号表单弹窗（name=null 为新建）。保存成功 → true；取消 → false */
export function openAccountDialog(name: string | null): Promise<boolean> {
  return mountDialog(AccountDialog, { name })
}

/** 把弹窗组件挂到 body 上，等它的 close 事件拿到结果后卸载 */
function mountDialog(component: Component, props: Record<string, unknown>): Promise<boolean> {
  return new Promise((resolve) => {
    const container = document.createElement('div')
    document.body.appendChild(container)
    const vnode = createVNode(component, {
      ...props,
      onClose: (result: boolean) => {
        render(null, container)
        container.remove()
        resolve(result)
      },
    })
    render(vnode, container)
  })
}
