<script setup lang="ts">
import { getCurrentWindow } from '@tauri-apps/api/window'

/** @tauri-apps/api 的 ResizeDirection 未导出，按其声明原样复刻 */
type ResizeDirection = 'East' | 'North' | 'NorthEast' | 'NorthWest' | 'South' | 'SouthEast' | 'SouthWest' | 'West'

const edges: { direction: ResizeDirection; side: string }[] = [
  { direction: 'North', side: 'n' },
  { direction: 'South', side: 's' },
  { direction: 'West', side: 'w' },
  { direction: 'East', side: 'e' },
  { direction: 'NorthWest', side: 'nw' },
  { direction: 'NorthEast', side: 'ne' },
  { direction: 'SouthWest', side: 'sw' },
  { direction: 'SouthEast', side: 'se' },
]

/** 去掉系统边框后 Linux 没有系统缩放边，靠这圈透明边角补；非 Tauri 环境静默 */
function startResizing(direction: ResizeDirection) {
  try {
    void getCurrentWindow()
      .startResizeDragging(direction)
      .catch(() => {})
  } catch {
    // getCurrentWindow 在非 Tauri 环境会同步抛错
  }
}
</script>

<template>
  <div class="resize-edges" aria-hidden="true">
    <span
      v-for="edge in edges"
      :key="edge.side"
      class="edge"
      :class="edge.side"
      @mousedown.prevent="startResizing(edge.direction)"
    />
  </div>
</template>

<style scoped>
.resize-edges {
  position: absolute;
  inset: 0;
  z-index: 40;
  pointer-events: none;
}

.edge {
  position: absolute;
  display: block;
  pointer-events: auto;
}

.edge.n {
  top: 0;
  left: 8px;
  right: 8px;
  height: 5px;
  cursor: ns-resize;
}

.edge.s {
  bottom: 0;
  left: 8px;
  right: 8px;
  height: 5px;
  cursor: ns-resize;
}

.edge.w {
  left: 0;
  top: 8px;
  bottom: 8px;
  width: 5px;
  cursor: ew-resize;
}

.edge.e {
  right: 0;
  top: 8px;
  bottom: 8px;
  width: 5px;
  cursor: ew-resize;
}

.edge.nw {
  top: 0;
  left: 0;
  width: 12px;
  height: 12px;
  cursor: nwse-resize;
}

.edge.ne {
  top: 0;
  right: 0;
  width: 12px;
  height: 12px;
  cursor: nesw-resize;
}

.edge.sw {
  bottom: 0;
  left: 0;
  width: 12px;
  height: 12px;
  cursor: nesw-resize;
}

.edge.se {
  bottom: 0;
  right: 0;
  width: 12px;
  height: 12px;
  cursor: nwse-resize;
}
</style>
