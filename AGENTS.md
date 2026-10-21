## Agent skills

> 模板来源：project-docs-workflow v5（模板演进后可重跑幂等 init 对齐；本文件可按项目实际修改）

### Issue tracker

工作条目位于 `docs/issues/<类型>/{human,agent}/`（类型：bug / feature / chore / suggestion）。任何工作开始前，先在对应子目录创建或更新条目，明确任务与责任，再动手；约定见 `docs/agents/issues.md`。

### Domain docs

单 context：仓库根目录 `CONTEXT.md`，ADR 位于 `docs/adr/`。约定见 `docs/agents/domain.md`。

### Development workflow

推荐五步流程：规划 / 骨架 / 细节 / 异常 / 回顾。角色与写入权限：用户掌范围与批准，编排者维护 PRD、与用户共同编写 SPEC 并拥有 `docs/toc.yml`，编码 agent 读 SPEC 实现并在收尾更新 Current。开工前读 SPEC 与 Current 作为事实源、PRD 理解范围与场景；实现细节以 SPEC 为准，范围 / 意图以 PRD 为准；发布目录由 `docs/toc.yml` 决定。步骤记录在 issue 的 `## 步骤`。约定见 `docs/agents/workflow.md`。
