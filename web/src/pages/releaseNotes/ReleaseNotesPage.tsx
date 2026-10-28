import { useMemo, useState } from 'react';
import { Megaphone, Search } from 'lucide-react';
import changelog from '../../../../CHANGELOG.md?raw';
import { parseChangelog } from '../../lib/changelog';
import { Markdown } from '../dashboards/widgets/markdown';
import './releaseNotes.css';

/** Release Notes: what changed in each ObserveCraft release, from CHANGELOG.md. */
export function ReleaseNotesPage() {
  const releases = useMemo(() => parseChangelog(changelog), []);
  const [text, setText] = useState('');
  const [scope, setScope] = useState<string | null>(null);
  const scopes = useMemo(() => [...new Set(releases.flatMap((r) => r.sections.flatMap((s) => s.items.map((i) => i.scope).filter((x): x is string => !!x))))].sort(), [releases]);
  const t = text.trim().toLowerCase();
  const shown = releases
    .map((r) => ({
      ...r,
      sections: r.sections
        .map((s) => ({ ...s, items: s.items.filter((i) => (!scope || i.scope === scope) && (!t || `${i.scope ?? ''} ${i.text}`.toLowerCase().includes(t))) }))
        .filter((s) => s.items.length),
    }))
    .filter((r) => r.sections.length);
  return (
    <div className="rn-page">
      <header className="product-header">
        <div className="product-title">
          <Megaphone size={20} /> Release Notes
        </div>
      </header>
      <div className="rn-body">
        <aside className="rn-filters">
          <label className="tv-filter">
            <Search size={15} />
            <input placeholder="Search changes" value={text} onChange={(e) => setText(e.target.value)} />
          </label>
          <div className="rn-filter-title">Product area</div>
          <button className={`rn-scope${scope === null ? ' active' : ''}`} onClick={() => setScope(null)}>
            All
          </button>
          {scopes.map((s) => (
            <button key={s} className={`rn-scope${scope === s ? ' active' : ''}`} onClick={() => setScope(s)}>
              {s}
            </button>
          ))}
        </aside>
        <main className="rn-releases">
          {shown.length === 0 && <p className="muted">No changes match.</p>}
          {shown.map((r) => (
            <article key={r.version} className="rn-release">
              <h2>
                {r.url ? (
                  <a href={r.url} target="_blank" rel="noreferrer">
                    v{r.version}
                  </a>
                ) : (
                  `v${r.version}`
                )}
                {r.date && <span className="muted rn-date">{new Date(`${r.date}T00:00:00`).toLocaleDateString(undefined, { year: 'numeric', month: 'long', day: 'numeric' })}</span>}
              </h2>
              {r.sections.map((s) => (
                <section key={s.title}>
                  <h3 className={/breaking/i.test(s.title) ? 'rn-breaking' : undefined}>{s.title.replace(/^⚠\s*/, '')}</h3>
                  <ul>
                    {s.items.map((i, n) => (
                      <li key={n}>
                        {i.scope && <span className="rn-chip">{i.scope}</span>}
                        <span className="rn-text">
                          <Markdown source={i.text} />
                        </span>
                      </li>
                    ))}
                  </ul>
                </section>
              ))}
            </article>
          ))}
        </main>
      </div>
    </div>
  );
}
