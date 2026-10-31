import { useEffect, useRef, useState, type ReactNode } from 'react';
import { Check, ChevronDown } from 'lucide-react';
import './ui.css';

/** Open state + close on outside click / Escape. */
export function usePopover() {
  const [open, setOpen] = useState(false);
  const ref = useRef<HTMLDivElement>(null);
  useEffect(() => {
    if (!open) return;
    const onDown = (e: MouseEvent) => {
      if (!ref.current?.contains(e.target as Node)) setOpen(false);
    };
    const onKey = (e: KeyboardEvent) => e.key === 'Escape' && setOpen(false);
    document.addEventListener('mousedown', onDown);
    document.addEventListener('keydown', onKey);
    return () => {
      document.removeEventListener('mousedown', onDown);
      document.removeEventListener('keydown', onKey);
    };
  }, [open]);
  return { open, setOpen, ref };
}

export interface Option<T extends string> {
  value: T;
  label: ReactNode;
  icon?: ReactNode;
  hint?: ReactNode;
}

interface SelectProps<T extends string> {
  value: T;
  options: Option<T>[];
  onChange: (value: T) => void;
  /** Custom trigger; defaults to a bordered button showing the selected label. */
  trigger?: (selected: Option<T> | undefined) => ReactNode;
  triggerClassName?: string;
  /** Floating label on the top border ("Data Source", "Assignee", …). */
  label?: string;
  align?: 'left' | 'right';
  title?: string;
  /** A search box above the options (long lists such as tag values). */
  searchable?: boolean;
}

/** Text an option is searched by: its label when it's a string, else its value. */
function optionText(o: Option<string>) {
  return `${typeof o.label === 'string' ? o.label : ''} ${o.value}`.toLowerCase();
}

/** Single-choice dropdown. */
export function Select<T extends string>({ value, options, onChange, trigger, triggerClassName, label, align = 'left', title, searchable }: SelectProps<T>) {
  const { open, setOpen, ref } = usePopover();
  const [search, setSearch] = useState('');
  const selected = options.find((o) => o.value === value);
  const needle = search.trim().toLowerCase();
  const shown = needle ? options.filter((o) => optionText(o).includes(needle)) : options;
  const pick = (v: T) => {
    onChange(v);
    setOpen(false);
    setSearch('');
  };
  return (
    <div className={`dd${label ? ' dd-labelled' : ''}`} ref={ref}>
      {label && <span className="dd-floating-label">{label}</span>}
      <button
        type="button"
        title={title}
        className={triggerClassName ?? 'dd-trigger'}
        onClick={(e) => {
          e.stopPropagation();
          setOpen((o) => !o);
        }}
      >
        {trigger ? (
          trigger(selected)
        ) : (
          <>
            {selected?.icon}
            <span className="dd-trigger-label">{selected?.label}</span>
            <ChevronDown size={14} className="dd-caret" />
          </>
        )}
      </button>
      {open && (
        <ul className={`dd-menu dd-${align}${searchable ? ' dd-searchable' : ''}`} onClick={(e) => e.stopPropagation()}>
          {searchable && (
            <li className="dd-search">
              <input autoFocus placeholder="Search…" value={search} onChange={(e) => setSearch(e.target.value)} onKeyDown={(e) => e.key === 'Enter' && shown.length > 0 && pick(shown[0].value)} />
            </li>
          )}
          {searchable && shown.length === 0 && <li className="dd-empty">No matches</li>}
          {shown.map((o) => (
            <li key={o.value}>
              <button type="button" className={o.value === value ? 'selected' : ''} onClick={() => pick(o.value)}>
                {o.icon}
                <span className="dd-item-label">{o.label}</span>
                {o.hint && <span className="dd-hint">{o.hint}</span>}
                {o.value === value && <Check size={14} className="dd-check" />}
              </button>
            </li>
          ))}
        </ul>
      )}
    </div>
  );
}

export interface MenuItem {
  label: ReactNode;
  icon?: ReactNode;
  onClick: () => void;
  disabled?: boolean;
}

/** Action menu ("Actions ▾", "More…"). */
export function Menu({ trigger, items, align = 'right', triggerClassName = 'btn' }: { trigger: ReactNode; items: MenuItem[]; align?: 'left' | 'right'; triggerClassName?: string }) {
  const { open, setOpen, ref } = usePopover();
  return (
    <div className="dd" ref={ref}>
      <button type="button" className={triggerClassName} onClick={() => setOpen((o) => !o)}>
        {trigger}
      </button>
      {open && (
        <ul className={`dd-menu dd-${align}`}>
          {items.map((it, i) => (
            <li key={i}>
              <button
                type="button"
                disabled={it.disabled}
                onClick={() => {
                  it.onClick();
                  setOpen(false);
                }}
              >
                {it.icon}
                <span className="dd-item-label">{it.label}</span>
              </button>
            </li>
          ))}
        </ul>
      )}
    </div>
  );
}
