<script setup lang="ts">
import { computed, watch } from 'vue'
import { applyUiStyle, type UiStyle } from '../../lib/api'
import { t } from '../../lib/i18n'
import { useSettingsDraft } from '../../composables/useSettingsDraft'

/**
 * 可选界面风格。
 *
 * 卡片结构完全一致，只有预览图配色不同（由 .preview.<value> 决定），
 * 所以统一成一份数据驱动；新增一套风格只需要在这里加一行 + 一个 CSS 块。
 */
const STYLE_OPTIONS = computed<{ value: UiStyle; name: string; desc: string }[]>(() => [
  { value: 'win11', name: t('settings.style1'), desc: t('settings.style1Desc') },
  { value: 'deepin', name: t('settings.style2'), desc: t('settings.style2Desc') },
  { value: 'mac', name: t('settings.style3'), desc: t('settings.style3Desc') },
])

const draft = useSettingsDraft()

function pick(style: UiStyle) {
  draft.uiStyle = style
  // watch 只在取值变化时触发；点已选中的卡片也要能恢复预览
  applyUiStyle(style)
}

// 风格切换即时预览整套界面，不用等保存
watch(
  () => draft.uiStyle,
  (value) => applyUiStyle(value),
)
</script>

<template>
  <div class="card block">
    <h2>{{ t('settings.style') }}</h2>
    <div class="styles">
      <button
        v-for="option in STYLE_OPTIONS"
        :key="option.value"
        type="button"
        class="style-card"
        :class="{ on: draft.uiStyle === option.value }"
        @click="pick(option.value)"
      >
        <span class="preview" :class="option.value">
          <span class="pv-side">
            <i class="pv-nav on"></i>
            <i class="pv-nav"></i>
            <i class="pv-nav"></i>
          </span>
          <span class="pv-main">
            <i class="pv-card"></i>
            <i class="pv-btn"></i>
          </span>
        </span>
        <span class="style-name">{{ option.name }}</span>
        <span class="hint">{{ option.desc }}</span>
      </button>
    </div>
    <p class="hint" style="margin: 2px 0 0">{{ t('settings.stylePreview') }}</p>
  </div>
</template>

<style scoped>
/* 三张风格卡片用响应式网格均分；窄窗口自动换列 */
.styles {
  display: grid;
  grid-template-columns: repeat(auto-fit, minmax(min(196px, 100%), 1fr));
  gap: 12px;
}

.style-card {
  min-width: 0;
  display: flex;
  flex-direction: column;
  align-items: flex-start;
  gap: 3px;
  padding: 10px;
  background: var(--panel);
  border: 1px solid var(--line-strong);
  border-radius: var(--radius);
  text-align: left;
  /* button 全局是 nowrap；这里带说明文字，必须允许换行，否则会溢出卡片 */
  white-space: normal;
}

/* 说明文字撑满卡片宽度并允许任意位置断行，避免长中文/英文溢出边框 */
.style-card .hint {
  align-self: stretch;
  line-height: 1.45;
  overflow-wrap: anywhere;
}

.style-card:hover {
  border-color: var(--accent);
}

.style-card.on {
  border-color: var(--accent);
  box-shadow: 0 0 0 1px var(--accent);
}

.style-name {
  font-weight: 500;
  margin-top: 2px;
}

/* 缩略示意图刻意不跟主题变量走：每套预览各画各的风格，切换前后都得是"目标风格的样子" */
.preview {
  position: relative;
  display: flex;
  width: 100%;
  height: 62px;
  border-radius: 6px;
  overflow: hidden;
  border: 0.5px solid rgba(0, 0, 0, 0.1);
}

.style-card.on .preview::after {
  content: "✓";
  position: absolute;
  top: 5px;
  right: 5px;
  width: 15px;
  height: 15px;
  border-radius: 50%;
  background: var(--accent);
  color: #fff;
  font-size: 10px;
  line-height: 15px;
  text-align: center;
}

.pv-side {
  width: 34%;
  display: flex;
  flex-direction: column;
  gap: 4px;
  padding: 8px 6px;
}

.pv-main {
  flex: 1;
  display: flex;
  flex-direction: column;
  gap: 6px;
  padding: 9px 8px;
}

.pv-nav {
  display: block;
  height: 7px;
}

.pv-card {
  display: block;
  height: 22px;
}

.pv-btn {
  display: block;
  width: 34px;
  height: 11px;
}

.preview.win11 {
  background: #f3f3f3;
}

.preview.win11 .pv-side {
  background: #ffffff;
  border-right: 1px solid #e5e5e5;
}

.preview.win11 .pv-nav {
  background: #e5e5e5;
  border-radius: 2px;
}

.preview.win11 .pv-nav.on {
  background: #e8f1fb;
  border-left: 2px solid #0067c0;
}

.preview.win11 .pv-card {
  background: #ffffff;
  border: 1px solid #e5e5e5;
  border-radius: 3px;
}

.preview.win11 .pv-btn {
  background: #0067c0;
  border-radius: 2px;
}

.preview.deepin {
  background: #eef1f6;
}

.preview.deepin .pv-side {
  background: rgba(255, 255, 255, 0.85);
  border-right: 1px solid #e4e8ee;
}

.preview.deepin .pv-nav {
  background: #dde3ec;
  border-radius: 999px;
}

.preview.deepin .pv-nav.on {
  background: #2b79ff;
}

.preview.deepin .pv-card {
  background: #ffffff;
  border-radius: 6px;
  box-shadow: 0 2px 6px rgba(35, 55, 95, 0.18);
}

.preview.deepin .pv-btn {
  background: #2b79ff;
  border-radius: 5px;
  box-shadow: 0 1px 3px rgba(43, 121, 255, 0.4);
}

/* macOS：侧栏压深一档、选中是蓝色小圆角、卡片只有一层极淡描影。
   与 Deepin 预览的差别刻意做在「侧栏底色 + 选中项形状」上，
   否则两套都蓝都白，缩略图根本分不出来。 */
.preview.mac {
  background: #f5f5f7;
}

.preview.mac .pv-side {
  position: relative;
  /* 顶上留出一条给交通灯，否则会压在第一个导航条上 */
  padding-top: 17px;
  background: #eceef1;
  border-right: 1px solid #e3e3e8;
}

/* 缩略图里最认得出的就是这三颗圆点，比配色差异有效得多 */
.preview.mac .pv-side::before {
  content: "";
  position: absolute;
  left: 7px;
  top: 7px;
  width: 5px;
  height: 5px;
  border-radius: 50%;
  background: #ff5f57;
  box-shadow: 7px 0 0 #febc2e, 14px 0 0 #28c840;
}

.preview.mac .pv-nav {
  background: #dfdfe4;
  border-radius: 4px;
}

.preview.mac .pv-nav.on {
  background: #007aff;
}

.preview.mac .pv-card {
  background: #ffffff;
  border-radius: 8px;
  box-shadow: 0 1px 3px rgba(0, 0, 0, 0.09);
}

.preview.mac .pv-btn {
  background: #007aff;
  border-radius: 6px;
}
</style>
