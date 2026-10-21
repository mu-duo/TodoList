> 模板来源：project-docs-workflow v5（模板演进后可重跑幂等 init 对齐；本文件可按项目实际修改）

# 开发流程（推荐）

本文件是推荐的单 issue 开发流程；项目可按实际覆盖。核心：小步推进、随时可 review、与用户对齐后再动手。

## 角色与写入权限

- **用户**：意图、范围与批准；评审 PRD、SPEC 与 ADR。可直接改文风，事实性修正走 agent。
- **编排者（orchestrator）**：维护 PRD；与用户共同编写 SPEC；评审 PRD / SPEC diff；拥有 `docs/toc.yml`；触发发布；把编码工作交给 coder。
- **编码 agent（coder）**：读 SPEC 实现；收尾时更新 Current 事实；可通过 issue 提议 SPEC 变更；**不**直接改 PRD / SPEC / TOC。
- **单 agent 模式**：一个 agent 可同时担任编排者与 coder，但角色边界仍然适用——换帽子时要显式切换。

## 工作单元

一条 issue = 一个可独立交付的工作单元（通常 feature 级；bug / chore 同构，形态相应简化）。

粒度建议（非强制）：切到「可单次 review 的推进单元」，≈0.5 人天仅启发式参考，按任务弹性调整。工作如何切分由 agent 自行感知，阶段不落盘（必要时可在当前 issue 内记一句）。

## 读什么

- **意图与场景**：PRD——产品 `docs/product/00-README.md`、`01-requirements.md`、`02-architecture.md`，再读模块 PRD（`docs/product/<NN-模块>/00-README.md`）。只用于理解范围与目标，不是实现事实源。
- **实现事实**：SPEC（`01-spec` / `02-interfaces` / `03-nfr` / `design/`）。
- **现状**：Current（`docs/current/`），只看实际建成的能力与限制。
- **发布**：`docs/toc.yml` 决定发布哪些文档。
- **冲突**：实现细节以 SPEC 为准，范围 / 意图以 PRD 为准；仍无法判定时升级给编排者 / 用户。

## 开工前对齐

读 SPEC 目标与 Current 现状，列出差距候选（模块 / FR、类型、切片、完成判据），与用户确认要做哪条并落 issue。差异只是候选：可能是漂移 / 质量 / 未验证 / 目标过时，不都是缺失功能。

- 编排者与用户共同确认范围并编写 / 对齐 SPEC；coder 按 SPEC 实现。范围或意图变化时先更新 PRD。
- 对齐范围内 agent 自主实现（遵守 ADR / 契约 / 安全）；高成本、破坏性或难以回退的决定先与用户确认。
- 对话确认即可：不冻结、不设仪式行。
- 需求变化时主动重对齐，并更新 issue 内的计划与完成判据。
- 方向有歧义 → 调用 `grilling`（未安装则按 `docs/agents/deps/grilling.md` 执行）；`grill-me` 仅入口别名。
- 不要吞并超出对齐范围的工作。

## 变更协议

- **PRD 变更**（范围 / 意图 / 目标用户 / 典型场景 / 非目标）：与用户对齐后更新 PRD；需求变化时同步更新 SPEC；人类 review PRD diff。coder 不直接改写 PRD。
- **SPEC 变更**（实现细节 / FR 语义 / 接口 / NFR）：由用户 + 编排者共同编写；coder 可通过 issue 提议；人类 / 编排者 review 后再实现。
- **Current 变更**：coder 在收尾时按选定基线更新事实；编排者 review。
- **ADR**：记录带理由与取舍的决策；ADR 是平级文档，不是流程中的一环。
- **冲突**：实现细节以 SPEC 为准，范围以 PRD 为准；无法调和时升级。
- 原型 / spike 在方向确认前豁免。
- 已实现的细节不会自动提升进 PRD 或 SPEC。

## PRD 影响触发（4 项）

收尾时（以及规划范围变更时）逐项检查；不设逐 FR 元数据，也不新增跟踪产物。

1. PRD 引用到的 FR 发生语义变化 → review / 更新 PRD。
2. 新增或变更的 FR 改变用户可见行为 → 新增 / 更新 PRD。
3. 纯内部改动 → PRD 不动，记一行 Log。
4. 目标用户 / 典型场景 / 范围 / 非目标变化 → 更新 PRD。

## 范围收敛（默认偏好）

- 用户所称「功能空间」即 PRD 约束下的状态空间 / 解空间，不另立概念。
- 触发条件 = 是否改变功能 / 范围目标；PRD / ADR 层（定义目标的地方）和 issue 规划层都适用。
- 未被 PRD 覆盖的新功能候选 = 扩张：提出前先用一句话说明理由。实现既有 PRD 的候选 = 收敛 / 实现，不需要额外论证。
- bug 修复和 chore 恢复 / 维持既有行为，豁免；若实际改变功能集合，应按 feature 重新分类。
- PRD 的非目标 / out-of-scope 是范围边界，不是否定决策；保留，不逐条解释理由。
- 这是默认偏好，不是门禁；不新增产物、不阻断开工；项目可覆盖。
- 记录策略：默认不记被拒备选方案；见 `../adr/README.md`（否定决策）与 `issues.md`（`-drop` 归档）。

## 五步流程（规划 / 骨架 / 细节 / 异常 / 回顾）

### 规划

编排者 + 用户确认范围并对齐 SPEC；coder 细化完成判据与步骤状态，给出 1–2 句方案。产出最小，不新开章节。差距分析在此完成（对照 SPEC 与 `docs/current/`；候选带模块 / FR、类型、切片、完成判据；用户挑选后才落 issue）。PRD 只作人类理解参考。

### 骨架

实现最小可运行的垂直切片（忽略输出 / 格式细节），留下一条运行证据，并把证据展示给用户（首个 review 点）。纯修复可跳过（在 Log 注明理由）；feature / 结构性改动必做。

### 细节

主路径实现、细节打磨与代码质量。`tdd` 可作内部循环（可选；未安装则按 `docs/agents/deps/tdd.md` 执行）。

### 异常

两点含义：作为流程步骤，主路径完成后再处理失败路径、边界与稳定性；作为开发现实，真实的开发阻塞立即处理并记 Log，不排队到本步。

### 回顾（收尾回顾）

issue 完成时做一次轻量的文档同步清单（coder 更新 Current 事实并提议 SPEC 变更；编排者维护 PRD、评审 PRD / SPEC / Current，模块或附录变化时更新 TOC）：

- **PRD 检查**：对照上面 4 项影响触发；需要改动则由编排者更新 PRD 并展示 diff，否则记一行「PRD 无变化」的 Log。
- **SPEC 检查**：coder 通过 issue 提议必要的 SPEC 变更；用户 + 编排者确认后落盘。coder 不直接改 SPEC。
- **Current 检查**：确认变更已包含在选定基线内；是则就地更新 `docs/current/` 对应文件（只改事实，不插入指向 issue 的自链接或任务历史），否则在 issue 记待同步项；编排者 review Current diff。
- 其余：`adr`、`context`、`issues`、`agents` 按需更新；全无更新时一行 Log 即可。

仅做文档同步；会话复盘属于 `retro`（未安装则按 `docs/agents/deps/boundaries.md` 执行）。

## 进度与续接

- 步骤记录在 issue 的 `## 步骤` 清单（就地更新）；仅在关键节点（范围变化 / 阻塞 / 交接 / 放弃）记 Log，不需要逐步机械追加。
- session 为软约束：session = 用户注意力窗口，节奏由用户掌握；agent 不得吞并超出对齐范围的工作。
- 恢复时先读 issue 的 `## 步骤` 与 `## Log`，与用户确认续点后再继续。

## 目录（TOC）维护与发布

- `docs/toc.yml` 是发布来源。模块或附录变化时由编排者 / agent 更新；用户也可直接编辑。所有路径必须存在，且落在 `docs/product`、`docs/current`、`docs/adr` 或根 `CONTEXT.md` 之内。
- 发布是**手动、流程之外**的动作：PRD / SPEC / Current 变更后，用户可触发 `docs-html-view` 构建多页 PRD。文档编辑不自动构建，init 也不创建任何发布产物。

## 边界

- 高风险 / 大规模多阶段 → `deepwork`（未安装则按 `docs/agents/deps/boundaries.md` 执行），不叠加。
- `tdd` 可作细节内循环（seam 确认并入规划对齐；未安装则按 `docs/agents/deps/tdd.md` 执行）。
- `grilling` 为对齐访谈（`grill-me` 仅入口别名；未安装则按 `docs/agents/deps/grilling.md` 执行）。
- 没有产品文档的既有项目 → 需用户同意的 bootstrap；旧版文档 → 迁移。两者都不会自动插入本流程。

## 相关

- Issue 体系：`issues.md`
- 产品 PRD 封面：`../product/00-README.md`
- 产品需求：`../product/01-requirements.md`
- 架构与模块划分：`../product/02-architecture.md`
- 发布目录源：`../toc.yml`
- 现状文档集（Current）：`../current/00-README.md`
- 依赖兜底文档（外部技能未安装时）：`docs/agents/deps/`
- 文档 HTML 发布（可选、手动）：独立技能 `docs-html-view`；本流程不自动安装 / 构建 / 渲染。
