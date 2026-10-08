> 模板来源：project-docs-workflow v2（模板演进后可重跑幂等 init 对齐；本文件可按项目实际修改）

# Issue 体系（仓库内工作跟踪）

工作条目的存放与流转规范。**任何工作（人 / agent）开始前，先在对应子目录创建或更新条目，明确任务与责任，再动手。**

## 目录结构

```
docs/issues/
├── bug/{human,agent}/        # 缺陷修复
├── feature/{human,agent}/    # 新功能
├── chore/{human,agent}/      # 维护 / 重构 / 杂务
├── suggestion/{human,agent}/ # 改进建议（无 history、无状态；归宿 adr / pdr）
└── <bug|feature|chore>/history/   # 归档：文件名尾部带 -done（完成）/ -drop（放弃）
```

- `human/` 与 `agent/` = 当前责任方；责任移交时把文件 `git mv` 到对方目录。
- 仅保留近期待办（约一周窗口）；完成或放弃的条目由完成者随手归档，人定期巡检。
- `suggestion` 特殊：处理完成后——采纳 → 结论落 adr（或更新 pdr），随后删除本条目；不采纳 → 仅当 human 明确要求时才落否决 ADR，否则直接删除；本质是 bug / feature / chore 的 → 挪到对应目录。

## 条目格式

文件名：项目语言短标题，无编号（例：`修复-首屏加载卡顿.md`）。归档时在末尾加状态后缀：`…-done.md` / `…-drop.md`。

```md
# <标题>
> Status: open｜doing｜blocked ｜ 责任: human｜agent ｜ 创建: YYYY-MM-DD ｜ 关联: ADR-XXXX / 其他条目

<背景与目标>

## 完成判据
- …

## 步骤
- [ ] 规划（完成判据与方案已对齐）
- [ ] 骨架（最小可运行切片 + 证据；纯修复可跳过并注明理由）
- [ ] 细节（主路径实现与质量）
- [ ] 异常（失败路径、边界与稳定性）
- [ ] 回顾（收尾回顾：文档同步）

## 结果（完成后填写，可选）
- …

## Log
- YYYY-MM-DD ……
```

（若本机安装了 project-docs-workflow：条目骨架 ~/.agents/skills/project-docs-workflow/assets/templates/entries/issue-entry.md；未安装则按本文格式创建。）

- `Status` 就地更新；`## Log` 追加式记录，不改历史行。
- 涉及 adr / pdr 的改动，须同步更新对应文档。
- 推进流程见 `workflow.md`（推荐五步：规划 / 骨架 / 细节 / 异常 / 回顾）；步骤状态就地更新，变更记 Log。

## 与开发流程的衔接

- 开工前与用户对齐本次范围与粒度（差距候选来自 PDR 目标与现状）。
- 步骤按 `docs/agents/workflow.md` 推荐流程推进，状态就地更新，变更记 Log。
- 收尾回顾时更新受影响模块的 `04-current.md`。

## 相关

- ADR 规则与目录：`../adr/README.md`
- 域文档约定：`domain.md`
- 开发流程（推荐）：`workflow.md`
