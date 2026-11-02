// A numbered, collapsible step of a setup form (Reference Tables → New, AWS → Add account).
import type { ReactNode } from 'react';
import { ChevronDown, ChevronRight } from 'lucide-react';

/** One collapsible step of the form. */
export function Step({ n, title, open, onToggle, done, children }: { n: number; title: string; open: boolean; onToggle: () => void; done?: boolean; children: ReactNode }) {
  return (
    <section className={`rt-step${open ? ' open' : ''}${done ? ' done' : ''}`}>
      <button type="button" className="rt-step-head" onClick={onToggle}>
        <span className="rt-step-num">{n}</span>
        {open ? <ChevronDown size={18} /> : <ChevronRight size={18} />}
        {title}
      </button>
      {open && <div className="rt-step-body">{children}</div>}
    </section>
  );
}
