import { useRef, useState } from 'react';
import { Braces } from 'lucide-react';
import { usePopover } from '../../../components/ui/Dropdown';
import type { Variable } from './variables';

/** The variable picker: lists what the field can read and inserts `{{ path }}` (or the bare path). */
export function VariablePicker({ variables, onPick, bare = false }: { variables: Variable[]; onPick: (text: string) => void; bare?: boolean }) {
  const { open, setOpen, ref } = usePopover();
  const [q, setQ] = useState('');
  const t = q.trim().toLowerCase();
  const shown = variables.filter((v) => !t || `${v.path} ${v.hint}`.toLowerCase().includes(t));
  const groups = [...new Set(shown.map((v) => v.group))];
  return (
    <div className="wfv" ref={ref}>
      <button type="button" className="wfv-btn" title="Insert a variable" aria-label="Insert a variable" onClick={() => setOpen((o) => !o)}>
        <Braces size={13} />
      </button>
      {open && (
        <div className="wfv-menu">
          <input className="wf-input" autoFocus placeholder="Search variables" value={q} onChange={(e) => setQ(e.target.value)} />
          <div className="wfv-list">
            {groups.map((g) => (
              <div key={g}>
                <div className="wfv-group">{g}</div>
                {shown
                  .filter((v) => v.group === g)
                  .map((v) => (
                    <button
                      type="button"
                      key={v.path}
                      className="wfv-item"
                      onClick={() => {
                        onPick(bare ? v.path : `{{ ${v.path} }}`);
                        setOpen(false);
                      }}
                    >
                      <span className="mono">{v.path}</span>
                      <span className="wf-muted">{v.hint}</span>
                    </button>
                  ))}
              </div>
            ))}
            {shown.length === 0 && <div className="wf-muted wfv-none">Nothing matches</div>}
          </div>
        </div>
      )}
    </div>
  );
}

interface Props {
  label: string;
  value: string;
  onChange: (v: string) => void;
  variables: Variable[];
  multiline?: boolean;
  required?: boolean;
  placeholder?: string;
  hint?: string;
  error?: string | null;
  /** Expressions: insert paths without braces. */
  bare?: boolean;
  mono?: boolean;
}

/** A labelled template input with a variable picker that inserts at the cursor. */
export function TemplateField({ label, value, onChange, variables, multiline, required, placeholder, hint, error, bare, mono }: Props) {
  const el = useRef<HTMLInputElement & HTMLTextAreaElement>(null);
  const insert = (text: string) => {
    const node = el.current;
    const at = node?.selectionStart ?? value.length;
    const end = node?.selectionEnd ?? at;
    onChange(value.slice(0, at) + text + value.slice(end));
    requestAnimationFrame(() => {
      node?.focus();
      node?.setSelectionRange(at + text.length, at + text.length);
    });
  };
  const cls = `wf-input${mono ? ' mono' : ''}${error ? ' wf-input-error' : ''}`;
  return (
    <label className="wf-field">
      <span className="wf-field-label">
        {label}
        {required && <span className="wf-required">*</span>}
        <VariablePicker variables={variables} onPick={insert} bare={bare} />
      </span>
      {multiline ? (
        <textarea ref={el} className={cls} rows={4} value={value} placeholder={placeholder} onChange={(e) => onChange(e.target.value)} />
      ) : (
        <input ref={el} className={cls} value={value} placeholder={placeholder} onChange={(e) => onChange(e.target.value)} />
      )}
      {error ? <span className="wf-field-error">{error}</span> : hint && <span className="wf-field-hint">{hint}</span>}
    </label>
  );
}
