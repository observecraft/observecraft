import { useState, type ReactNode } from 'react';
import { ChevronDown, ChevronRight } from 'lucide-react';
import { IntegrationLogo } from '../integrations/IntegrationLogo';

/** A home page section: icon, title, optional right-side actions, body. */
export function HomeSection({ id, icon, title, actions, children }: { id: string; icon: ReactNode; title: string; actions?: ReactNode; children: ReactNode }) {
  return (
    <section id={id} className="ph-card" aria-labelledby={`${id}-title`}>
      <header className="ph-card-head">
        <span className="ph-card-icon">{icon}</span>
        <h2 id={`${id}-title`}>{title}</h2>
        {actions && <div className="ph-card-actions">{actions}</div>}
      </header>
      {children}
    </section>
  );
}

export interface ProviderTab<K extends string> {
  key: K;
  label: string;
  /** Logo registry key; a React node for our own sources. */
  logo: string | ReactNode;
}

/** Provider tabs with logos (GitHub | GitLab | Azure DevOps, Jira | Linear | Work Items). */
export function ProviderTabs<K extends string>({ tabs, value, onChange }: { tabs: ProviderTab<K>[]; value: K; onChange: (k: K) => void }) {
  return (
    <div className="ph-provider-tabs" role="tablist">
      {tabs.map((t) => (
        <button key={t.key} role="tab" aria-selected={t.key === value} className={`ph-provider-tab${t.key === value ? ' active' : ''}`} onClick={() => onChange(t.key)}>
          <span className="ph-provider-logo">{typeof t.logo === 'string' ? <IntegrationLogo logo={t.logo} name={t.label} size={14} /> : t.logo}</span>
          {t.label}
        </button>
      ))}
    </div>
  );
}

/** A collapsible group of rows with a count badge. */
export function HomeGroup({ label, count, defaultOpen = true, children }: { label: string; count: number; defaultOpen?: boolean; children: ReactNode }) {
  const [open, setOpen] = useState(defaultOpen);
  return (
    <div className="ph-group">
      <button className="ph-group-head" aria-expanded={open} onClick={() => setOpen(!open)}>
        {open ? <ChevronDown size={14} /> : <ChevronRight size={14} />}
        <span>{label}</span>
        <span className="ph-count">{count}</span>
      </button>
      {open && children}
    </div>
  );
}

/** Blurred placeholder rows behind a centred card: what a section shows once its provider is connected. */
export function ConnectOverlay({
  logo,
  name,
  title,
  text,
  action,
  onAction,
  children,
}: {
  logo: string;
  name: string;
  title: string;
  text: string;
  action: string;
  onAction: () => void;
  children: ReactNode;
}) {
  return (
    <div className="ph-overlay-wrap">
      <div className="ph-blurred" aria-hidden="true">
        {children}
      </div>
      <div className="ph-overlay-card" role="group" aria-label={title}>
        <span className="ph-overlay-logo">
          <IntegrationLogo logo={logo} name={name} size={26} />
        </span>
        <strong>{title}</strong>
        <p>{text}</p>
        <button className="btn btn-primary" onClick={onAction}>
          {action}
        </button>
      </div>
    </div>
  );
}

/** Coloured priority dot plus its name. */
export function Priority({ level, label }: { level: string; label: string }) {
  if (level === 'none' && !label) return <span className="sc-muted">—</span>;
  return (
    <span className="ph-priority">
      <span className={`ph-dot ph-dot-${level}`} />
      {label || 'None'}
    </span>
  );
}
