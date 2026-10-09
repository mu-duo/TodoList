# 设置面板与主题（暗色 / 亮色 / 跟随系统）

> Status: doing ｜ 责任: agent ｜ 创建: 2026-10-09 ｜ 关联: ADR-0006（设置存储）；grilling 定案 + @librarian 事实核证

## 共识
- **入口**：header 最右**齿轮图标**（刷新 → 置顶 → 设置）；形态＝**模态卡片**（带遮罩；点遮罩 / Esc 关闭；Enter 不处理）。
- **内容**：主题三选一（跟随系统 / 亮色 / 暗色）＋ 一行只读版本号。
- **机制**：`setTheme('dark'|'light'|null)`；**一套** `prefers-color-scheme` CSS 覆盖三态（钉死/跟随由它决定）；CSS 加 `color-scheme`；**首帧前应用主题**（防闪白）；「跟随系统」订阅 `onThemeChanged`（带 guard 防回环，需实测）。
- **暗色**：覆盖**全套设计令牌**（背景 / 文字 / 边框 / 卡片 / 危险红 / 强调色），保对比度，不新增其它视觉风格。
- **权限**：新增 `core:window:allow-set-theme`（**不在** `core:default`，缺失会调用失败）。
- **持久化**：**应用配置目录**下的 `settings.yaml`（Windows：`%APPDATA%\com.lftan.todolist\settings.yaml`，见 ADR-0006）——**不进数据目录**，避免机器相关偏好污染 git / 网盘同步；写盘即时（防抖）。

## 完成判据
- 齿轮按钮 → 模态设置面板；三态切换**立即生效**，重启后保持。
- 暗色下既有界面全部可读（卡片 / 清单条 / 详情 / 抽屉 / warning / error / 危险红），对比度达标。
- 「跟随系统」下改 Windows 应用主题 → 界面**实时**跟随。
- 首帧不闪白。

## 已知与注意
- Windows 上 `theme()` 读的是**「应用主题」`AppsUseLightTheme`**，且**高对比度开启时返回 light**。
- `setTheme` 依赖 WebView2 `ICoreWebView2_13`（runtime ≥ 101.0.1210.39），过旧会**静默无效**。
- `prefers-color-scheme` 一旦被 `setTheme` 钉死，就不再随系统变化。

## 步骤
- [x] 规划（grilling + 事实核证）
- [x] 骨架（权限 / 设置命令 / 齿轮 + 模态）
- [x] 细节（三态 / 暗色令牌 / 防闪白 / 实时跟随）
- [x] 异常（settings 读取失败回退 / 非法值 / 回环 guard / 首帧）
- [x] 回顾（文档同步）

## Log
- 2026-10-09 grilling 定案（形态＝模态卡片 / 三态 / 实时跟随 / 暗色＝全套令牌）；@librarian 核证主题 API、`setTheme` 权限与 `prefers-color-scheme` 钉死机制。
- 2026-10-09 写 ADR 时发现 `02-interfaces` 既有契约（非任务数据不进数据目录）与"放数据目录"的方案冲突，且数据目录可能进用户同步；用户改选**应用配置目录** → ADR-0006、契约条目、RELEASE 说明已按此落定。
- 2026-10-09 实现交付：新增 `src/theme.ts`、`src/components/SettingsModal.tsx`；header 齿轮、启动门控（首帧前定主题）、`setTheme` + **单一** `prefers-color-scheme` 暗色令牌；Rust 侧 `load_settings` / `save_settings`（应用配置目录）与 `core:window:allow-set-theme`。
- 2026-10-09 **CDP 真机验证全部 PASS**：启动屏如期退出（未卡首帧）→ 齿轮 → 模态；切「暗色」后 `documentElement.colorScheme=dark` 且 `%APPDATA%\com.lftan.todolist\settings.yaml` 落盘 `theme: dark`、`actionError=null`（权限未被拒）；Esc 关模态。
- 2026-10-09 用户肉眼复核通过（暗色对比度达标）；回顾：`docs/pdr/10-app/04-current.md`、ADR-0006、`02-interfaces` 均已同步；条目归档 -done。
