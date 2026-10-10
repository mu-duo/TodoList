# 窄屏：详情按 Esc 返回后，任务描述出现焦点边框

> Status: doing ｜ 责任: agent ｜ 创建: 2026-10-10 ｜ 关联: 子任务编辑与勾选（同批发现）；**待诊断**

## 现象（用户报告）
窄屏下在任务详情中按 `Esc` 返回主页面时，**任务的描述部分出现边框效果**（疑似焦点环 / `:focus-visible`）。

## 待诊断（CDP 真机取证）
- `Esc` 关闭详情后焦点落在哪个元素（`document.activeElement`）——是否为任务行或行内描述元素；
- 该边框来自 UA 默认 `:focus-visible` 还是项目自定义样式（列 `outline` / `box-shadow` 的 computed 值与命中规则）；
- 宽屏是否同样存在（只是被 hover 样式掩盖）；
- 关闭详情时是否缺少"把焦点还给触发元素"的逻辑。

## 完成判据
- 窄屏 `Esc` 返回后，任务行 / 描述**不出现意外焦点边框**；
- **但键盘用户仍需保留可见的焦点指示**——不得粗暴 `outline: none` 一刀切；
- 宽屏行为一致（要么同样不出现，要么两屏一致）。

## 步骤
- [x] 记录（用户报告 + 落条目）
- [x] 诊断（CDP 真机取证 → 根因）
- [x] 修复（最小改动）
- [x] 验证（窄 / 宽双屏 + 键盘可见性）
- [x] 回顾（文档同步）

## Log
- 2026-10-10 用户报告，落条目并派 CDP 诊断。
- 2026-10-10 **诊断（CDP 真机）根因（RC3，极高置信）**：`.task-open` 是 `<button>`（`App.tsx` 任务行渲染处）；鼠标点行会聚焦它。Esc 由全局 window 监听关闭详情（`App.tsx:766-775`），但**不移动 / 清除焦点**；焦点仍在按钮上，而 Esc 属键盘交互 → 命中 `button:focus-visible{outline:2px solid var(--accent);outline-offset:2px}`（`App.css:219-222`）。**宽屏同样存在**（宽屏 Esc 后该行已非 `is-selected` 却仍有边框 → 证明是焦点环而非选中边框）。
- 2026-10-10 **修复方向**：闭合详情时按**输入模态**处理——指针打开 → 清理该行按钮焦点（`blur`）；键盘打开 → **保留**焦点与焦点环（键盘用户本该看到）；**禁止**全局 `outline:none`。可用 `click` 事件的 `detail === 0` 判定"由键盘激活"。
- 2026-10-10 **修复交付**：`openTask(taskId, viaKeyboard)` 以 `click` 的 `detail === 0` 判定键盘激活并记入 `openedByKeyboardRef`；`closeDetail` 仅在**指针打开**时 `blur()` 该行按钮（`App.tsx:585-590`）；键盘打开**保留**焦点与焦点环；未用全局 `outline: none`。
- 2026-10-10 **CDP 复验 PASS**：指针打开 → Esc 后 `activeElement=BODY`、`.task-open:focus-visible=false`（无意外边框）；键盘打开（Tab + Enter）→ Esc 后焦点仍在 `.task-open` 且 `:focus-visible=true`（焦点环保留）；宽 / 窄一致。
- 2026-10-10 用户验收通过；同源路径（指针点击 + 回车）另行立条目修复；**条目归档 -done**。
