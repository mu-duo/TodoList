# 回退：边界类 skill（deepwork / retro / improve-codebase-architecture）

> 内部回退（精简版），裁剪 / 改编自 [mattpocock/skills](https://github.com/mattpocock/skills) 的 deepwork / retro / improve-codebase-architecture（MIT License, Copyright (c) 2026 Matt Pocock）。若环境已安装对应完整 skill，请优先使用完整版。

> 当某个边界类 skill 被引用但环境未安装时，按对应小节执行。

## deepwork

用于成本高、风险大、需跨多个阶段推进的整块工作。与 `project-docs-workflow` 的五步流程（规划 / 骨架 / 细节 / 异常 / 回顾）**互斥**：同一个 effort 只能选一个，绝不叠加。

**无完整 deepwork 时**：退回到一个最小的分阶段计划，每个阶段设评审闸门——**计划 → 实现 → 评审**，逐阶段推进。把每个阶段当作一次可评审的推送，评审通过才进入下一阶段。

## retro

会话/agent 回顾的所有权。第五步「回顾」只是文档同步，不在此列。

**最小三问回顾**：

1. **发生了什么**：读指定会话的原始记录（未指定则默认当前会话）。
2. **保留什么**：哪些做法或环境配置有效。
3. **改变什么**：给出针对 agent **环境**的改进候选（导航指针、自动化检查、编码规范、全局指令瘦身、工具经济性、多余指令、信息可达性），按严重程度排序呈现给用户。

## improve-codebase-architecture

代码库架构改进的路线（找出「深化模块」的机会，把浅模块变深，提升可测试性与 AI 可导航性），建立在领域模型与共享架构词汇（module / interface / depth / seam / adapter / leverage / locality）之上。

**无完整流程时**：本技能包不提供扫描与 HTML 报告流程；需要术语或 ADR 支持时，**转 `domain-modeling`**（未安装则见同目录 `domain-modeling.md`）。

## doc-opt

`doc-opt` 随包附带于 `skills/doc-opt`（完整版）；环境未安装时可参考同目录 `doc-opt.md` 的最小回退。
