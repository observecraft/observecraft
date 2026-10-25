import { useState, type ReactNode } from 'react';
import { ChevronDown, ChevronRight, PanelLeftClose, PanelLeftOpen, type LucideIcon } from 'lucide-react';
import './section.css';

/** Collapsible card for one panel section; `id` is the scroll-spy anchor. */
export function CollapsibleSection({ id, title, icon: Icon, children, extra }: { id: string; title: string; icon: LucideIcon; children: ReactNode; extra?: ReactNode }) {
  const [open, setOpen] = useState(true);
  return (
    <section className="hp-section" data-section={id} id={`hp-${id}`}>
      <header>
        <button className="hp-section-toggle" onClick={() => setOpen((o) => !o)}>
          {open ? <ChevronDown size={18} /> : <ChevronRight size={18} />}
          <Icon size={18} />
          <span>{title}</span>
        </button>
        {extra}
      </header>
      {open && <div className="hp-section-body">{children}</div>}
    </section>
  );
}

export interface NavItem {
  id: string;
  label: string;
  icon: LucideIcon;
}

/** Sticky left nav for a page of CollapsibleSections; highlights `active`, can collapse to icons. */
export function SectionNav({ items, active, onJump, collapsed, onToggle }: { items: NavItem[]; active: string; onJump: (id: string) => void; collapsed: boolean; onToggle: () => void }) {
  return (
    <nav className={`hp-nav${collapsed ? ' collapsed' : ''}`}>
      {items.map((s) => (
        <button key={s.id} className={active === s.id ? 'active' : ''} onClick={() => onJump(s.id)} title={s.label}>
          <s.icon size={16} />
          {!collapsed && <span>{s.label}</span>}
        </button>
      ))}
      <button className="hp-nav-collapse" onClick={onToggle} title={collapsed ? 'Expand' : 'Collapse'}>
        {collapsed ? <PanelLeftOpen size={16} /> : <PanelLeftClose size={16} />}
        {!collapsed && <span>Collapse</span>}
      </button>
    </nav>
  );
}

/** Scroll spy: the section (data-section) nearest the top of `root` is active. */
export function activeSection(root: HTMLElement, fallback: string): string {
  const top = root.getBoundingClientRect().top;
  let current = fallback;
  for (const el of root.querySelectorAll<HTMLElement>('[data-section]')) {
    if (el.getBoundingClientRect().top - top <= 140) current = el.dataset.section!;
  }
  return current;
}
