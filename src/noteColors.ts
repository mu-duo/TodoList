/**
 * 便签配色（与主题联动：亮 / 暗两套 6 档调色板，按「档位序号」对应重映射）。
 *
 * 2026-10-16 最终调整（用户细化 + 视觉层对比度校准）：
 * - 亮色主题：浅色系 黄 / 白 / 绿 / 蓝 / 青 / 紫（t1..t6 依次对应此序）；
 * - 暗色主题：从灰到灰白 6 个度（t1 最灰 → t6 灰白；t4 起底色转浅，文字色翻转为深色）。
 *
 * 暗色 ramp 的校准依据（正文目标：AA 起步、尽量 AAA）：
 * 中灰带（约 #60–#98）在两个文字方向上都不可能达到 7:1——#7A7A7A 上压纯白也只有 ≈4.2:1，
 * 因此 t3 / t4 必须让开这一段，两族各自等距（相邻档 ΔL* ≈ 8–9，翻转处 ΔL* ≈ 34）：
 *   t1..t3 = #2E2E2E / #404040 / #525252，浅字 → 正文 11.6 / 9.2 / 7.3:1；
 *   t4..t6 = #A8A8A8 / #C0C0C0 / #D8D8D8，深字 → 正文  7.3 / 8.7 / 10.1:1。
 * 视觉层所有浓淡（图标 / 把手 / hover / 选区 / 滚动条）都由 text 色按 alpha 派生，
 * 故正文对比度即这套色板的对比度上限；以上取值让最弱点仍 ≥ 7.3:1。
 * 另：亮暗两套各含一张接近纯白的卡片（light t2 / dark t6），它是纸颗粒强度的上限约束。
 *
 * 契约：`NOTE_TONES` 顺序即轮换取色顺序；`NOTE_COLOR_COUNT` 供窗口创建侧取模。
 * 视觉层（NoteApp）消费 `NOTE_COLORS[scheme][tone]`；色值可微调、字段可扩展，
 * 但导出名与「同一序号在亮/暗两套调色板间对应」的映射关系保持稳定。
 */

export type NoteScheme = "light" | "dark";

export type NoteTone = "t1" | "t2" | "t3" | "t4" | "t5" | "t6";

/** 轮换取色顺序（相邻便签必不同档）。 */
export const NOTE_TONES: readonly NoteTone[] = [
  "t1",
  "t2",
  "t3",
  "t4",
  "t5",
  "t6",
];

/** 档位数量；窗口创建侧取模用。 */
export const NOTE_COLOR_COUNT = NOTE_TONES.length;

export type NoteColorStyle = {
  /** 卡片底色。 */
  bg: string;
  /** 卡片描边。 */
  border: string;
  /** 文本与图标主色。 */
  text: string;
};

export const NOTE_COLORS: Record<NoteScheme, Record<NoteTone, NoteColorStyle>> = {
  // 亮色：浅色系 黄 / 白 / 绿 / 蓝 / 青 / 紫 —— 深色文字。
  light: {
    t1: { bg: "#FDF3C8", border: "#E7D98F", text: "#4A4326" }, // 黄
    t2: { bg: "#FFFFFF", border: "#E4E4E4", text: "#383838" }, // 白
    t3: { bg: "#DAF0DD", border: "#A9D6B3", text: "#2E4634" }, // 绿
    t4: { bg: "#D8E9F8", border: "#A9CAE4", text: "#2C3E4E" }, // 蓝
    t5: { bg: "#D2F0F2", border: "#9FD6DA", text: "#274347" }, // 青
    t6: { bg: "#E8E0F7", border: "#C9B9E6", text: "#3C3252" }, // 紫
  },
  // 暗色：灰 → 灰白 6 度 —— t1..t3 浅色文字，t4 起底色转浅、文字翻转为深色。
  // 中灰带（#60–#98）无法在任一方向达到 7:1，故 t3/t4 让开该段（详见文件头说明）。
  dark: {
    t1: { bg: "#2E2E2E", border: "#3E3E3E", text: "#EDEDED" },
    t2: { bg: "#404040", border: "#505050", text: "#F1F1F1" },
    t3: { bg: "#525252", border: "#626262", text: "#F7F7F7" },
    t4: { bg: "#A8A8A8", border: "#B8B8B8", text: "#1A1A1A" },
    t5: { bg: "#C0C0C0", border: "#D0D0D0", text: "#222222" },
    t6: { bg: "#D8D8D8", border: "#E8E8E8", text: "#2A2A2A" },
  },
};
