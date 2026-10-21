> 模板来源：project-docs-workflow v5（模板演进后可重跑幂等 init 对齐；本文件可按项目实际修改）

# Domain Docs

工程类技能在探索本代码库时应如何消费本仓库的领域文档。

本模板覆盖单 context 布局；多 context（CONTEXT-MAP.md）暂未纳入。

## Before exploring, read these

- 根目录的 **`CONTEXT.md`**。
- **`docs/adr/`**：读取与将要改动的领域相关的 ADR；规则与目录见 `docs/adr/README.md`。

如果这些文件不存在，**静默继续**：不要提示缺失，也不要主动建议创建它们。`domain-modeling`（未安装则按 `docs/agents/deps/domain-modeling.md` 执行）会在词条或决策真正定案时按需创建。

外部技能未安装时的兜底流程物化在 `docs/agents/deps/`。**组合回退**：`grill-with-docs` 是 `grilling` + `domain-modeling` 的包装（术语确定即写入 `CONTEXT.md`，满足三条判据记 ADR）；未安装该包装时，按 `docs/agents/deps/grilling.md` 与 `docs/agents/deps/domain-modeling.md` 交织执行。`improve-codebase-architecture` 未安装时按 `docs/agents/deps/boundaries.md` 执行。以上均为**可选的独立技能名，不代表本包已随附**；本 skill 不会自动安装它们。

## File structure

按项目实际结构填写（单 context 仓库的典型布局如下，据实替换）：

```
/
├── CONTEXT.md
├── docs/
│   ├── toc.yml
│   ├── adr/
│   ├── agents/
│   ├── common/
│   ├── current/
│   ├── issues/
│   └── product/
└── <源码目录…>
```

## Use the glossary's vocabulary

当你的产出提到某个领域概念（issue 标题、重构提案、假设、测试名）时，使用 `CONTEXT.md` 中定义的术语。不要漂移到术语表明确回避的同义词。

如果你需要的概念还不在术语表里，这本身是一个信号：要么你在发明项目不使用的语言（重新考虑），要么存在真实缺口（记下来，交给 `domain-modeling`；未安装则按 `docs/agents/deps/domain-modeling.md` 执行）。

## Flag ADR conflicts

如果你的产出与既有 ADR 冲突，显式指出，不要默默覆盖：

> _与 ADR-XXXX（<决策短标题>）冲突，但值得重新讨论，因为……_
