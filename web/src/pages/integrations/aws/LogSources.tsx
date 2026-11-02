// The log sources the Datadog Forwarder subscribes to: searchable switches with All / Enabled / Disabled
// and Enable All / Disable All (add-account flows and the account's Log Collection tab).
import { useState } from 'react';
import { Search } from 'lucide-react';

type Filter = 'all' | 'on' | 'off';

export function LogSources({ services, value, onChange }: { services: { id: string; name: string }[]; value: string[]; onChange: (sources: string[]) => void }) {
  const [query, setQuery] = useState('');
  const [filter, setFilter] = useState<Filter>('all');
  const on = new Set(value);
  const q = query.trim().toLowerCase();
  const shown = services.filter((s) => (!q || s.name.toLowerCase().includes(q)) && (filter === 'all' || (filter === 'on') === on.has(s.id)));
  const toggle = (id: string) => onChange(on.has(id) ? value.filter((v) => v !== id) : [...value, id]);
  const counts: Record<Filter, number> = { all: services.length, on: services.filter((s) => on.has(s.id)).length, off: services.filter((s) => !on.has(s.id)).length };
  return (
    <div className="aws-sources">
      <label className="aws-search">
        <Search size={15} />
        <input value={query} onChange={(e) => setQuery(e.target.value)} placeholder="Search" />
      </label>
      <div className="aws-sources-bar">
        <div className="aws-segments" role="tablist">
          {(['all', 'on', 'off'] as const).map((f) => (
            <button key={f} type="button" role="tab" aria-selected={filter === f} className={filter === f ? 'active' : ''} onClick={() => setFilter(f)}>
              {{ all: 'All', on: 'Enabled', off: 'Disabled' }[f]} ({counts[f]})
            </button>
          ))}
        </div>
        <span className="aws-bulk">
          <button type="button" className="link-btn" onClick={() => onChange(services.map((s) => s.id))}>
            Enable All
          </button>
          <button type="button" className="link-btn" onClick={() => onChange([])}>
            Disable All
          </button>
        </span>
      </div>
      <ul className="aws-source-list">
        {shown.map((s) => (
          <li key={s.id}>
            <span>{s.name}</span>
            <button type="button" role="switch" aria-checked={on.has(s.id)} aria-label={s.name} className={`switch${on.has(s.id) ? ' on' : ''}`} onClick={() => toggle(s.id)} />
          </li>
        ))}
        {shown.length === 0 && <li className="muted">No log sources match.</li>}
      </ul>
    </div>
  );
}
