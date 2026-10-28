<script setup lang="ts">
import { computed, ref } from 'vue'
import { Modal as AModal } from '@arco-design/web-vue'
import type { ButtonProps } from '@arco-design/web-vue/es/button'
import '@arco-design/web-vue/es/modal/style/css.js'
import { t } from '../lib/i18n'

/**
 * 通用二次确认框（Arco Modal 外壳）。
 *
 * 只做"确认/取消"这一件事：需要输入内容的场景用各自的表单弹窗。
 * 一般不在模板里挂它，走 lib/dialog.ts 的 confirmDialog() 函数式调用；
 * 结果等关窗动画播完才由 close 带出，宿主据此卸载，退场动画不会被掐断。
 */
const props = defineProps<{
  title: string
  body: string
  /** 确认按钮文案；不传就用「确定」 */
  confirmText?: string
  /** true 时确认按钮用危险色：会中断当前动作的操作都该是红的 */
  danger?: boolean
}>()

const emit = defineEmits<{ close: [result: boolean] }>()

const visible = ref(true)
const result = ref(false)

/** 危险确认走红色主按钮，其余用默认主色 */
const okButtonProps = computed<ButtonProps | undefined>(() =>
  props.danger ? { status: 'danger' } : undefined,
)
</script>

<template>
  <AModal
    v-model:visible="visible"
    :title="title"
    :width="420"
    title-align="start"
    :ok-text="confirmText ?? t('common.confirm')"
    :cancel-text="t('common.cancel')"
    :ok-button-props="okButtonProps"
    @ok="result = true"
    @close="emit('close', result)"
  >
    <p class="muted">{{ body }}</p>
  </AModal>
</template>
