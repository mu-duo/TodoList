import type { ReactNode } from "react";

type Props = {
  /** 子板块 key，用于 id / data 属性。 */
  sectionKey: string;
  /** 板块标题。 */
  title: string;
  /** 右侧说明（如「随改随存」）。 */
  hint?: ReactNode;
  /** content 是否加边框包裹。 */
  boxed?: boolean;
  collapsed: boolean;
  onToggle: () => void;
  children: ReactNode;
};

/**
 * 任务详情的统一子板块：`h3 > button` 标题行 + 可折叠 content。
 *
 * 折叠时**不渲染** content（不用 `hidden` 占位），避免留下无主的高度和焦点；
 * 标题行整行可点，chevron 在收起时旋转 -90°。
 */
export function DetailSection({
  sectionKey,
  title,
  hint,
  boxed = false,
  collapsed,
  onToggle,
  children,
}: Props) {
  return (
    <section className="detail-section" data-section={sectionKey}>
      <h3 className="detail-section-head" id={`detail-${sectionKey}-head`}>
        <button type="button" aria-expanded={!collapsed} onClick={onToggle}>
          <span className="detail-section-chevron" aria-hidden="true">
            <svg
              viewBox="0 0 24 24"
              fill="none"
              stroke="currentColor"
              strokeWidth="2"
              strokeLinecap="round"
              strokeLinejoin="round"
            >
              <path d="M6 9l6 6 6-6" />
            </svg>
          </span>
          <span className="detail-section-title">{title}</span>
          {hint ? <small className="detail-section-hint">{hint}</small> : null}
        </button>
      </h3>
      {collapsed ? null : (
        <div className={`detail-section-body${boxed ? " is-boxed" : ""}`}>
          {children}
        </div>
      )}
    </section>
  );
}
