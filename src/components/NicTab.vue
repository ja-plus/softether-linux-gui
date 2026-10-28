<script setup lang="ts">
import NicSettings from './settings/NicSettings.vue'

/**
 * 「虚拟网络适配器」页。
 *
 * 这一页没有任何要落盘的偏好——增删启停都由后端立刻写进客户端配置，
 * 数据源是 `overview.nics`（卡片自己会在挂载时 refresh 一次），
 * 所以这里不需要草稿，也不需要底部那条自动保存状态栏。
 */
</script>

<template>
  <section class="pane">
    <div class="pane-scroll">
      <NicSettings />
    </div>
  </section>
</template>
