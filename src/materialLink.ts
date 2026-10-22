/**
 * 「关联材料」中链接的客户端校验（与后端 `agent_set_task_intent` 口径一致）。
 *
 * 规则：trim 后非空、≤2048、无任何空白字符、必须 `http://` / `https://` / `file://`
 * 前缀（大小写不敏感）、不与已有材料重复。纯函数，方便 UI 就地提示与写入前兜底。
 */

export const MATERIAL_LINK_MAX_LENGTH = 2048;

const LINK_PREFIXES = ["http://", "https://", "file://"];

/** 归一化：仅去首尾空白，保留用户输入的大小写。 */
export function normalizeMaterialLink(raw: string): string {
  return raw.trim();
}

/**
 * 校验一条候选链接；返回中文错误文案，通过则返回 null。
 * `existing` 传当前全部材料（路径 + 链接），用于去重。
 */
export function materialLinkError(
  raw: string,
  existing: readonly string[],
): string | null {
  const value = normalizeMaterialLink(raw);
  if (!value) return "请输入链接。";
  if (value.length > MATERIAL_LINK_MAX_LENGTH) {
    return `链接过长（最多 ${MATERIAL_LINK_MAX_LENGTH} 个字符）。`;
  }
  if (/\s/.test(value)) return "链接不能包含空格或换行。";
  const lower = value.toLowerCase();
  if (!LINK_PREFIXES.some((prefix) => lower.startsWith(prefix))) {
    return "链接需以 http://、https:// 或 file:// 开头。";
  }
  if (existing.some((item) => item.trim().toLowerCase() === lower)) {
    return "该链接已在关联材料中。";
  }
  return null;
}
