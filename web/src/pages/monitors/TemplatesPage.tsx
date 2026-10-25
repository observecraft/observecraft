import { useState } from 'react';
import { Link } from 'react-router-dom';
import { ChevronRight, Gauge, Search } from 'lucide-react';
import { TEMPLATES } from './templates';
import './monitors.css';

export function TemplatesPage() {
  const [q, setQ] = useState('');
  const f = q.trim().toLowerCase();
  const shown = TEMPLATES.filter((t) => !f || `${t.source} ${t.title} ${t.description} ${t.query}`.toLowerCase().includes(f));
  return (
    <div className="mon-create">
      <header className="mon-crumbs">
        <Link to="/monitors" className="mon-crumb-root">
          <Gauge size={22} /> Monitors
        </Link>
        <ChevronRight size={18} className="muted" />
        <span>Monitor Templates</span>
      </header>
      <div className="mon-tpl-page">
        <label className="mon-tpl-search">
          <Search size={16} />
          <input autoFocus placeholder="Search templates by service, integration, or use case" value={q} onChange={(e) => setQ(e.target.value)} />
        </label>
        <div className="mon-tpl-grid">
          {shown.map((t) => (
            <Link key={t.id} className="mon-tpl-card" to={`/monitors/create/${t.type}?template=${t.id}`}>
              <span className="mon-tpl-source">{t.source}</span>
              <b>{t.title}</b>
              <span className="muted">{t.description}</span>
              <code>{t.query}</code>
            </Link>
          ))}
          {shown.length === 0 && <p className="muted">No template matches “{q}”.</p>}
        </div>
      </div>
    </div>
  );
}
