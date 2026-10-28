import { forwardRef, useImperativeHandle, useRef } from 'react';
import { highlightSql } from './sqlEditorModel';

/** Highlighted SQL, read-only (template cards, docs examples). */
export function SqlHighlight({ sql, className = '' }: { sql: string; className?: string }) {
  return (
    <pre className={`sqle-code ${className}`}>
      {highlightSql(sql).map((t, i) => (
        <span key={i} className={`sqle-tok-${t.kind}`}>
          {t.text}
        </span>
      ))}
    </pre>
  );
}

export interface SqlCodeEditorHandle {
  /** Inserts text at the caret (or replaces the selection) and focuses the editor. */
  insert: (text: string) => void;
}

interface Props {
  value: string;
  onChange: (sql: string) => void;
  /** Ctrl/Cmd+Enter. */
  onRun: () => void;
}

/**
 * A textarea over a highlighted copy of its text: the textarea's own text is transparent, so the
 * caret, selection and native editing stay while the colors come from the layer below.
 */
export const SqlCodeEditor = forwardRef<SqlCodeEditorHandle, Props>(function SqlCodeEditor({ value, onChange, onRun }, ref) {
  const area = useRef<HTMLTextAreaElement>(null);
  const layer = useRef<HTMLPreElement>(null);
  const gutter = useRef<HTMLDivElement>(null);

  useImperativeHandle(ref, () => ({
    insert(text: string) {
      const el = area.current;
      if (!el) return;
      const { selectionStart: start, selectionEnd: end } = el;
      onChange(value.slice(0, start) + text + value.slice(end));
      requestAnimationFrame(() => {
        el.focus();
        el.setSelectionRange(start + text.length, start + text.length);
      });
    },
  }));

  const syncScroll = () => {
    const el = area.current;
    if (!el) return;
    if (layer.current) {
      layer.current.scrollTop = el.scrollTop;
      layer.current.scrollLeft = el.scrollLeft;
    }
    if (gutter.current) gutter.current.scrollTop = el.scrollTop;
  };

  const onKeyDown = (e: React.KeyboardEvent<HTMLTextAreaElement>) => {
    if (e.key === 'Enter' && (e.metaKey || e.ctrlKey)) {
      e.preventDefault();
      onRun();
      return;
    }
    if (e.key === 'Tab' && !e.shiftKey) {
      e.preventDefault();
      const el = e.currentTarget;
      const { selectionStart: start, selectionEnd: end } = el;
      onChange(value.slice(0, start) + '  ' + value.slice(end));
      requestAnimationFrame(() => el.setSelectionRange(start + 2, start + 2));
    }
  };

  const lines = value.split('\n').length;
  return (
    <div className="sqle-editor">
      <div className="sqle-gutter" ref={gutter} aria-hidden>
        {Array.from({ length: lines }, (_, i) => (
          <div key={i}>{i + 1}</div>
        ))}
      </div>
      <div className="sqle-editor-text">
        <pre className="sqle-code sqle-layer" ref={layer} aria-hidden>
          {highlightSql(value).map((t, i) => (
            <span key={i} className={`sqle-tok-${t.kind}`}>
              {t.text}
            </span>
          ))}
          {'\n'}
        </pre>
        <textarea
          ref={area}
          className="sqle-code sqle-input"
          value={value}
          spellCheck={false}
          wrap="off"
          autoCapitalize="off"
          autoComplete="off"
          aria-label="SQL query"
          placeholder="SELECT … FROM hosts"
          onChange={(e) => onChange(e.target.value)}
          onScroll={syncScroll}
          onKeyDown={onKeyDown}
        />
      </div>
    </div>
  );
});
