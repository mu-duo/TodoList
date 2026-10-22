# 详情面板：关联材料独立成板块 + 可点击打开

> Status: done ｜ 责任: agent ｜ 创建: 2026-10-22 ｜ 关联: Agent协助任务、详情面板统一子板块与折叠

## 背景与目标

「关联材料」目前藏在详情面板的「Agent 协助」板块内，且只能添加/移除、不可点击。目标：把它提为独立子板块（`title + content`、可折叠、位于 Agent 协助 之前），并支持点击打开——本地路径在资源管理器中定位、URL 在浏览器中打开；材料仍作为 Agent 的只读授权根参与运行。

## 关键约束

- `agent.paths` 的沙箱校验（`sandbox::guard_source_root`）会拒绝 URL（含冒号即被判非法），因此 URL 必须另立字段 `agent.links`；`paths` 的语义与校验完全不变，Agent 读取逻辑零改动。
- `agent.links` **不传给 Agent**：保持「只读本地沙箱」，不新增网络能力。
- 打开策略（安全）：`http://` / `https://` / `file://` → 浏览器（`open_url`）；存在的目录 → 打开目录；存在的文件 → 资源管理器中**定位**（`reveal_item_in_dir`，不执行）；不存在 / 其他协议 → 报错。

## 完成判据

- 详情面板七块：标题 / 任务属性 / 子任务 / 备注 / 其他信息 / **关联材料** / Agent 协助；关联材料默认展开、可折叠，折叠 key 为 `materials`（并入 `detail.collapsed`）；
- 关联材料内：路径列表 + 链接列表（同一行样式、无类型图标；hover tooltip 说明点击行为），行右侧独立「移除 ×」（不触发打开），底部「添加文件 / 添加目录 / 添加链接」；
- 点击行：URL → 浏览器；目录 → 打开；文件 → 资源管理器定位；失败 → 顶部 `actionError` 提示「无法打开：…」；
- Agent 协助 不再出现任何材料 UI，保留启用开关 / 运行控制 / 运行历史；
- 添加链接：仅接受 `http://` / `https://` / `file://`（大小写不敏感），trim、无空白、≤2048，去重；
- `agent_set_task_intent` 携带 `links`；`TaskView.agent` 暴露 `links`；旧 YAML 无 `links` 时按空处理；
- `cargo test` 与 `pnpm build` 通过；接口文档同步 `agent.links`、`agent_open_external`、`detail.collapsed` 合法 key。

## 步骤

- [x] 规划（数据形状 / 打开策略 / 协议校验 / 交互 / 命名 / 展示形态已与用户逐条对齐）
- [x] 骨架（Rust `links` 字段与命令；前端新板块落位）
- [x] 细节（列表与添加入口、tooltip、折叠 key、样式统一）
- [x] 异常（非法协议、缺失路径、打开失败、旧 YAML 兼容）
- [x] 回顾（文档同步、条目归档）

## Log

- 2026-10-22 用户提出：关联材料从 Agent 协助 中独立出来，且可点击打开（路径→资源管理器，URL→浏览器）。对齐结论：新增 `agent.links` 与 `paths` 并列；URL 不喂给 Agent；打开策略为 `file→定位`（不执行）、目录→打开、http(s)/file→浏览器；板块 key `materials`、位于 Agent 协助 之前；行内无类型图标，用 hover tooltip 区分；添加链接用内联输入框。
- 2026-10-22 实施：Rust `TaskAgent.links` + `validate_links`（三类协议前缀、≤2048、禁空白、去重）+ `agent_open_external`（`classify_target` 纯函数；`tauri-plugin-opener` 的 `reveal_item_in_dir` 走 `SHOpenFolderAndSelectItems`，无 shell 拼接）；前端新增 `MaterialsPanel` 与 `materialLink.ts` 校验、`openExternal` 接线、`materials` 折叠板块；Agent 协助 摘除材料 UI。证据：`cargo test --lib` 242 passed / 0 failed、`pnpm build` 通过。
- 2026-10-22 收口：核对 links 在 5 个持久化调用点全部透传（否则会清空已有链接）；补 `TaskView.agent` 注释。待用户真机冒烟（点路径定位 / 点链接开浏览器 / 非法协议提示）后归档。
- 2026-10-22 用户验收通过；随后链接改为也供 Agent 抓取（另立条目），本次点击打开能力保持。归档。
