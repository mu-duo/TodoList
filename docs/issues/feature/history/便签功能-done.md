# 便签功能

> Status: done（2026-10-16 用户验收通过） ｜ 责任: agent ｜ 创建: 2026-10-15 ｜ 关联: grilling 定案（三轮+终稿）；多窗口架构、App Bar 新入口、capability 扩展

## 背景与目标

App Bar 新增「便签」动作入口：点击即新建一个独立便签窗口（Tauri 多窗口，本项目首次），可多实例；颜色与主题联动（亮/暗各 6 色轮换）；pin（默认关，可切换系统级置顶）与 close（丢弃内容、无提示、不持久化）。

## 共识（grilling 定案）

- **形态**：独立 Tauri 窗口（方案 A）；无边框、不进任务栏/Alt-Tab、默认 280×320、可鼠标缩放（最小 200×160）、双击顶部不最大化；文本区与按钮外按住可拖动。（2026-10-16 调整：固定尺寸 → 可缩放）
- **多开**：点击一次新建一个，无上限、无复用；首个居中，其后相对上一张右下 +28px 级联（上一张已关则回中心）；位置不跨重启记忆。
- **颜色**：`src/noteColors.ts` 两套 6 档调色板——亮色主题 = 浅色系 黄 / 白 / 绿 / 蓝 / 青 / 紫；暗色主题 = 灰 → 灰白 6 度（t1..t6）；按打开顺序轮换（相邻不同档）；主题切换按档位序号实时重映射；「跟随系统」按系统明暗。（2026-10-16 调整：6 色相马卡龙 → 黑白灰 → 本版：亮色彩系 + 暗色灰阶）
- **pin**：默认不置顶；toggle `setAlwaysOnTop`；按钮描边↔激活态区分。
- **close**：立即关闭、丢弃内容、无警告；Alt+F4 等同；Esc 不绑定。
- **内容**：多行纯文本、自动聚焦、无限制、无 placeholder、溢出显示滚动条。
- **生命周期**：不持久化。**关闭应用**时若存在便签 → 先确认：托盘设置关闭时的关窗 → 应用内确认框；托盘菜单「退出」→ 系统原生对话框（不恢复主窗、不抢焦点）。确认后全部便签一并关闭并退出应用；取消 → 一切保持；托盘设置**开启**时关窗仍为隐藏到托盘（便签保留）；应用退出即全部丢弃。（2026-10-16 调整：原「主窗口关闭不影响便签」仅保留托盘隐藏路径）
- **入口**：App Bar 新动作入口「便签」（内联 SVG：圆角便签纸+折角+两行线）；默认顺序 `refresh, pin, note, settings, agent`；已存自定义顺序按既有契约追加末尾，不改契约。
- **不做**：与 Todo 数据零联动、快捷键、圆角窗口（直角全出血卡片）、transparent 窗口。

## 契约（实现边界）

- URL：`/?note=<colorIndex>&theme=<pref>`；存在 `note` 参数即便签窗口；colorIndex 取模归一，theme ∈ system|light|dark（非法回退 system）。
- `NoteApp` props：`{ colorIndex: number; themePref: ThemePref }`（默认导出，`src/NoteApp.tsx`）。
- 事件：主窗口主题变更广播 `theme-pref-changed`（payload: ThemePref）；便签窗口监听并 `applyThemePref`。
- 颜色契约：`src/noteColors.ts` 导出 `NOTE_TONES / NOTE_COLOR_COUNT / NOTE_COLORS`（t1..t6 灰阶；色值可微调、字段可扩展，导出名稳定）。
- 窗口创建：`src/noteWindow.ts` 的 `openNoteWindow(themePref)`；标签 `note-<n>`；创建失败由调用方报错（actionError）。
- 退出确认：Rust 在「有便签的关闭请求」时 `prevent_close` 并广播 `exit-requested-with-notes`（payload `{ noteCount }`）；前端用 ConfirmModal 确认后 invoke `exit_app`（退出进程）；取消则一切保持。托盘「退出」路径不广播事件：直接弹系统原生确认对话框（非阻塞；不恢复主窗），确认 → `app.exit(0)`，取消 → 保持。（2026-10-16 调整：托盘路径由「恢复主窗 + 事件模态」改为原生对话框）

## 完成判据

- App Bar 出现「便签」入口，图标同风格；点击一次新建一个独立便签窗口，可多开。
- 便签窗口行为 / 颜色 / 内容 / pin / close / 生命周期均符合上述共识。
- 已保存自定义顺序的环境不受破坏（新入口按契约追加末尾）。
- 验证证据：`npx tsc --noEmit` 与 `pnpm build` 通过；`cargo check`（src-tauri）通过（含 capability 校验）；用户在 `pnpm tauri dev` 肉眼复核：多开、级联、切主题换色（灰度档）、pin、拖动、缩放、关闭丢弃、**退出确认（取消 / 确认两路）**、托盘/退出场景。
- 文档：CONTEXT.md「便签」术语；PDR（01-spec / 02-interfaces / 04-current）同步（回顾步）。

## 步骤

- [x] 规划（grilling 定案）
- [x] 骨架（窗口创建管线 + 便签壳渲染；构建/分包证据见 Log，运行复核待 dev 冒烟）
- [x] 细节（UI / 配色 / pin / close / 主题联动）
- [x] 异常（连点竞态串行化、首帧主题、创建失败 actionError、主题事件失败静默）
- [x] 回顾（PDR 与 CONTEXT 同步、用户验收、版本 1.8.0 与产物构建、归档）

## 结果

- 便签功能交付并**用户验收通过（2026-10-16）**：独立窗口多开；两套 6 档调色板（亮色 浅色系 黄/白/绿/蓝/青/紫、暗色 灰→灰白 6 度）与主题实时重映射；8 向缩放（最小 200×160、双击不最大化）；pin / close；退出确认（关窗应用内确认框 / 托盘原生对话框）。
- 实现面：`src/noteWindow.ts`、`src/NoteApp.tsx`、`src/note.css`、`src/noteColors.ts`、`src/appBar.ts`、`src/App.tsx`、`src/main.tsx`、`src/components/AppBar.tsx`、`src-tauri/src/lib.rs`、`src-tauri/capabilities/default.json`。
- 验证：`npx tsc --noEmit`、`pnpm build`（NoteApp 独立分包）、`cargo check`、`cargo test --all-targets` **234/234**、`cargo clippy --all-targets` 无警告（收尾修掉一处 `needless_return`）全通过；用户 dev 冒烟通过（多开 / 级联 / 换色 / 缩放 / pin / 拖动 / 关闭丢弃 / 退出确认场景 A·B / 托盘路径）。
- 版本与产物：并入 **1.8.0**（三处版本号经 `scripts/build-release.ps1` 同步）；产物已构建并收集——`release/TodoList_1.8.0_x64-setup.exe`（3,135,078 B）、`release/TodoList_1.8.0_x64.exe`（8,838,144 B），各带 `.sha256`。提交 / 打 tag / 推送与 Release 上传由用户执行。
- 文档：CONTEXT「便签」；PDR 01-spec §8/§10（FR-APP-35）、02-interfaces §5/§8、04-current 已同步。

## Log

- 2026-10-15 grilling 定案（方案 A 独立窗口）；建条，范围与判据与用户对齐。
- 2026-10-15 并行派发：@designer（便签 UI / 图标 / 配色微调，新文件为主）、@fixer（多窗口管线与权限）。主线收口验证。
- 2026-10-15 两路交付：@designer 交付 NoteApp/note.css/图标（含 12 组对比度复核，最低约 8.6:1），@fixer 交付 noteWindow 管线/接线/权限（Rust 零改动，已按 label 过滤确认）。修正两处：快速连点竞态（创建串行化）、首帧闪色（创建参数补 theme）。
- 2026-10-15 合流验证：`npx tsc --noEmit` 通过；`pnpm build` 通过且 NoteApp 独立分包（`NoteApp-*.js` / `NoteApp-*.css`）；`cargo check` 通过；capability 权限名已逐条对照 desktop-schema.json。待用户 `pnpm tauri dev` 肉眼验收。
- 2026-10-16 验收反馈两项调整：① 色系改黑白灰——`src/noteColors.ts` 重写为 6 档灰阶（`NOTE_HUES` → `NOTE_TONES`）；② 便签改为可鼠标缩放——默认 280×320、最小 200×160、双击不最大化，resize 热区 + 右下握把。两路并行收敛后重新构建复验。
- 2026-10-16 两项调整合流完成：灰阶适配（12 组对比度最低约 5.2:1，颗粒/高光/交互态重校准）；8 向 resize 热区 + 右下握把（与滚动条、按钮、拖动的冲突均有几何处理与源码核对；`startResizeDragging` / `ResizeDirection` 与类型定义逐字核对，双击最大化门控已从 crate 源码确证）。验证：`npx tsc --noEmit`、`pnpm build`（NoteApp 独立分包）、`cargo check` 全通过。待用户复验。
- 2026-10-16 色系再调整（用户细化）：亮色主题改回浅色系 黄/白/绿/蓝/青/紫（t1..t6 依此序）；暗色主题改为 灰→灰白 6 度（t4 起文字翻转为深色）。`noteColors.ts` 已重写；designer 按新方案复核颗粒/高光/交互态与对比度后复验。
- 2026-10-16 色系复核完成：暗色 ramp 按对比度数学重构（中灰带 #60–#98 在任一文字方向都无法达标 → t3/t4 让开该段，成「3 深灰 + 3 浅灰」两族；正文最低 7.29:1，全套派生层逐项复核）；亮色 6 色保持种子值；颗粒 0.06 统一，亮色顶部受光恢复 0.13，暗色受光改墨色同向渐变（跨 t4 翻转点单规则覆盖）。`npx tsc --noEmit` + `pnpm build` 通过。待用户复验（暗色档位分布的取舍已备备选方案）。
- 2026-10-16 色系通过（用户确认）；同时报告退出缺陷：托盘设置关闭时关主窗，进程被便签窗口挂着不退（僵尸态；主窗已销毁、单例唤醒无法恢复）。定案退出语义修正：关闭应用（托盘关闭时关窗 / 托盘菜单退出）时若存在便签 → 确认弹窗，确认 → 便签全关 + 退出，取消 → 保持；托盘开启时关窗仍隐藏到托盘。派发 fix-1（Rust 关闭拦截 + exit_app 命令 + 前端确认模态）。
- 2026-10-16 托盘「退出」确认方式定案（用户选择原生方案）：不再恢复主窗、不广播事件，改由 Rust 直接弹系统原生对话框（非阻塞回调；确认 → `app.exit(0)`，取消 → 保持；防连点）。关窗路径保持应用内确认框。派发 fix-1（Rust-only 增量）。
- 2026-10-16 托盘「退出」原生确认实现完成（fix-1，Rust-only）：`AtomicBool` 防连点 + 非阻塞原生对话框（`OkCancelCustom("退出","取消")`，不绑定父窗）；API 语义已对 tauri-plugin-dialog 2.8.2 源码核验（第一参数=确认、`show` 回调 bool=确认、失败无同步返回）。已知限制：若底层 `run_on_main_thread` 失败导致回调不触发，防连点开关保持，该路径不再弹（不可感知，记档）。`npx tsc --noEmit` + `pnpm build` + `cargo check` 全通过。待用户冒烟。
- 2026-10-16 **用户验收通过**；PDR 与 CONTEXT 同步；版本同步 1.8.0（发布脚本）、`cargo test --all-targets` **234/234**、clippy 收尾（修 `needless_return`）后产物重打并收集（`release/`）；条目归档 `feature/history/便签功能-done.md`；提交 / tag / 推送由用户执行。
