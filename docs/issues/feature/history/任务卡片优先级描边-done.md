# 任务卡片优先级描边
> Status: done ｜ 责任: agent ｜ 创建: 2026-10-29 ｜ 关联: FR-APP-06

任务列表卡片目前只用文字「优先级 高/中/低」表达优先级，扫视时缺少视觉分层。为高 / 中优先级补 1px 彩色描边，并把优先级文字移到卡片右侧单行显示；低优先级与无优先级不加描边。

## 完成判据

- 高优先级卡片边框 1px 红（亮 `#dc2626` / 暗 `#f26a6a`），中优先级 1px 黄（亮 `#e0a400` / 暗 `#f2c66d`）；颜色走新建令牌 `--priority-high` / `--priority-medium`
- 无优先级、`low`、空串、未知值（如 `urgent`）不加描边
- 完成态（`is-done`）与归档态（`is-archived`）不显示优先级描边
- 选中态（`is-selected`）的强调色描边压过优先级色；悬停保留优先级色，只变底色与阴影
- 优先级文字渲染在卡片右侧、单行不换行；卡片过窄时整块隐藏（不截断、不换行）
- 拖拽浮层、回收站行、详情面板不参与；仅任务列表卡片（普通列表、分组视图、已完成分组）
- 不加优先级底色、色条，不预留 soft 令牌
- `npm run build` 通过；人工目视：无/低、中、高、选中、悬停、完成/归档、暗色

## 步骤

- [x] 规划（完成判据与方案已对齐）
- [x] 骨架（纯样式改动，最小切片即完整实现；跳过独立骨架截图）
- [x] 细节（主路径实现与质量）
- [x] 异常（未知 priority / 状态叠加 / 暗色令牌 / 窄卡片隐藏）
- [x] 回顾（收尾回顾：文档同步）

## 结果（完成后填写，可选）

- `src/App.css`：新增 `--priority-high` / `--priority-medium`（亮 + 暗各一组）；`.task-card.is-priority-*` 只换 1px 描边色，用 `:not(.is-done):not(.is-archived):not(.is-selected)` 排除三态，悬停单独钉回优先级色；`.task-priority` 行尾单行文字（`pointer-events: none`）；`.task-card` 加 `container-type: inline-size`，`@container (max-width: 360px)` 下整块隐藏优先级文字。
- `src/App.tsx` `taskRowClass()`：按 `priority === "high" | "medium"` 精确追加类名；优先级文字从标题下方 meta 行移到右侧 `.task-actions` 内，meta 行出现条件改为 `due || showSource`。
- `docs/current/app.md` 与 `docs/product/10-app/01-spec.md`（FR-APP-06）同步列表描边与右侧文字规则；PRD 无变化。
- `npm run build` 通过（tsc + vite）。

## Log

- 2026-10-29 与用户完成 grilling 对齐（10 项决策）；范围限定为列表卡片描边，不改数据层与详情页。
- 2026-10-29 首版实现（2px 描边 + meta 行保留文字）与构建通过，issue 归档。
- 2026-10-29 用户复核后调整：描边改 1px；优先级文字移到卡片右侧、单行不换行；构建通过，文档同步后重新归档。
- 2026-10-29 归档卡片行尾优先级文字改为 `--text-faint`，与已弱化的归档标题一致；构建通过。
