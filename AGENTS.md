## Agent skills

> 模板来源：project-docs-workflow v2（模板演进后可重跑幂等 init 对齐；本文件可按项目实际修改）

### Issue tracker

工作条目位于 `docs/issues/<类型>/{human,agent}/`（类型：bug / feature / chore / suggestion）。任何工作开始前，先在对应子目录创建或更新条目，明确任务与责任，再动手；约定见 `docs/agents/issues.md`。

### Domain docs

单 context：仓库根目录 `CONTEXT.md`，ADR 位于 `docs/adr/`。约定见 `docs/agents/domain.md`。

### Development workflow

推荐五步流程：规划 / 骨架 / 细节 / 异常 / 回顾。开工前先读 PDR 目标与现状（各模块 `04-current.md`），与用户对齐本次范围与粒度；步骤记录在 issue 的 `## 步骤`。约定见 `docs/agents/workflow.md`。
