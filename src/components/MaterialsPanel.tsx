import { useEffect, useRef, useState } from "react";
import { materialLinkError, normalizeMaterialLink } from "../materialLink";

type MaterialsPanelProps = {
  /** 关联的本地文件 / 目录（原路径，仅本机使用）。 */
  paths: string[];
  /** 关联链接（仅用于点击打开，不喂给 Agent）。 */
  links: string[];
  onAddPaths: (directories: boolean) => void;
  onRemovePath: (path: string) => void;
  onAddLink: (url: string) => void;
  onRemoveLink: (url: string) => void;
  /** 点击材料行：文件 → 资源管理器定位、目录 → 打开、http(s)/file → 浏览器。 */
  onOpen: (target: string) => void;
};

/**
 * 详情「关联材料」板块的内容：本地路径 + 链接，统一按「可点击打开」的行展示。
 *
 * 链接的新增是就地单行输入（不开对话框）；输入态与错误提示都由本组件持有，
 * 父级用 `key={task.id}` 挂载，切换任务时自然清空、不串味。
 */
export function MaterialsPanel({
  paths,
  links,
  onAddPaths,
  onRemovePath,
  onAddLink,
  onRemoveLink,
  onOpen,
}: MaterialsPanelProps) {
  const [linkOpen, setLinkOpen] = useState(false);
  const [draft, setDraft] = useState("");
  const [error, setError] = useState<string | null>(null);
  const inputRef = useRef<HTMLInputElement>(null);

  useEffect(() => {
    if (linkOpen) inputRef.current?.focus();
  }, [linkOpen]);

  function closeLinkInput() {
    setLinkOpen(false);
    setDraft("");
    setError(null);
  }

  function submitLink() {
    const message = materialLinkError(draft, [...paths, ...links]);
    if (message) {
      // 校验失败：就地提示，不写盘、不收起
      setError(message);
      return;
    }
    onAddLink(normalizeMaterialLink(draft));
    closeLinkInput();
  }

  const isEmpty = paths.length === 0 && links.length === 0;

  return (
    <>
      {isEmpty ? (
        <p className="agent-hint">暂无关联材料。</p>
      ) : (
        <ul className="material-list">
          {paths.map((path) => (
            <li key={`path:${path}`} className="material-item">
              <button
                type="button"
                className="material-open"
                title={`点击在资源管理器中打开：${path}`}
                onClick={() => onOpen(path)}
              >
                {path}
              </button>
              <button
                type="button"
                className="material-remove"
                aria-label={`移除 ${path}`}
                title="移除"
                onClick={() => onRemovePath(path)}
              >
                ×
              </button>
            </li>
          ))}
          {links.map((link) => (
            <li key={`link:${link}`} className="material-item">
              <button
                type="button"
                className="material-open"
                title={`点击在浏览器中打开：${link}`}
                onClick={() => onOpen(link)}
              >
                {link}
              </button>
              <button
                type="button"
                className="material-remove"
                aria-label={`移除 ${link}`}
                title="移除"
                onClick={() => onRemoveLink(link)}
              >
                ×
              </button>
            </li>
          ))}
        </ul>
      )}

      <div className="material-add-row">
        <button
          type="button"
          className="row-action"
          onClick={() => onAddPaths(false)}
        >
          添加文件
        </button>
        <button
          type="button"
          className="row-action"
          onClick={() => onAddPaths(true)}
        >
          添加目录
        </button>
        <button
          type="button"
          className="row-action"
          aria-expanded={linkOpen}
          onClick={() => (linkOpen ? closeLinkInput() : setLinkOpen(true))}
        >
          添加链接
        </button>
      </div>

      {linkOpen && (
        <div className="material-link-field">
          <input
            ref={inputRef}
            className="material-link-input"
            value={draft}
            placeholder="https://…"
            aria-label="新增关联链接"
            aria-invalid={error ? true : undefined}
            onChange={(e) => {
              setDraft(e.currentTarget.value);
              if (error) setError(null);
            }}
            onKeyDown={(e) => {
              // IME 组合态一律不触发
              if (e.nativeEvent.isComposing || e.nativeEvent.keyCode === 229) {
                return;
              }
              if (e.key === "Enter") {
                // 隔离：避免触发详情面板的「Enter 保存并关闭」
                e.preventDefault();
                e.stopPropagation();
                submitLink();
              } else if (e.key === "Escape") {
                e.preventDefault();
                e.stopPropagation();
                closeLinkInput();
              }
            }}
          />
          {error && (
            <p className="material-link-error" role="alert">
              {error}
            </p>
          )}
        </div>
      )}

      <p className="agent-hint">
        本地路径与链接都会提供给 Agent 只读访问（链接仅支持 http(s) 抓取）。
      </p>
    </>
  );
}
