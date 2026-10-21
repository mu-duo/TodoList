# 回退：domain-modeling

> 当需要调用 `domain-modeling` 但环境未安装该 skill 时，按本文执行。

> 内部回退（精简版），裁剪自 [mattpocock/skills](https://github.com/mattpocock/skills) 的 domain-modeling（MIT License, Copyright (c) 2026 Matt Pocock）。若环境已安装同名完整 skill，请优先使用完整版。

主动构建并打磨项目的领域模型：挑战术语、构造边界场景、在术语与决策成型时立刻写下。**修改**模型才算本流程；只是为查词而读 `CONTEXT.md` 不算。

## 步骤

1. **定位文件**：根目录有 `CONTEXT-MAP.md` → 多上下文，按它找到相关 `CONTEXT.md`；只有根 `CONTEXT.md` → 单上下文；都没有 → 在第一个术语确定时惰性创建。
2. **质疑与锐化**：
   - 用词与 `CONTEXT.md` 冲突 → 立刻指出，问「到底是哪个」。
   - 用词含糊或一词多义 → 提出精确的规范术语。
   - 讨论领域关系 → 用具体场景压力测试，逼清概念边界。
   - 用户描述行为 → 与代码交叉核对，矛盾当面提出。
3. **当场更新 `CONTEXT.md`**（不要攒批），沿用下方格式。
4. **按需记录 ADR**，沿用下方规则与格式。

## CONTEXT.md 格式

```md
# {上下文名称}

{一两句话说明这是什么上下文、为什么存在。}

## Language

**Order**：
{一或两句话的术语描述}
_Avoid_: Purchase, transaction
```

- 有主见：多词指同一概念时选最好的一个，其余列入 `_Avoid_`。
- 定义紧致：最多一两句，说它**是什么**，不是**做什么**。
- 只收本项目特有概念；通用编程概念不进。
- 完全不含实现细节：它是词汇表，不是 spec。

## ADR 格式

位置 `docs/adr/`，顺序编号 `0001-slug.md`，取现有最大号加一。

```md
# {决策的简短标题}

{1-3 句：背景是什么、决定了什么、为什么。}
```

可选小节仅在确有价值时加：`Status`（`proposed | accepted | deprecated | superseded by ADR-NNNN`）、Considered Options、Consequences。

**仅当三条同时成立才写 ADR**：难以逆转；无背景会令人意外；是真实取舍的结果。缺一即跳过。
