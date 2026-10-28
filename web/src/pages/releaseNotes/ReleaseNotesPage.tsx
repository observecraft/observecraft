import { useMemo, useState } from 'react';
import { Link2 } from 'lucide-react';
import changelog from '../../../../CHANGELOG.md?raw';
import { parseChangelog } from '../../lib/changelog';
import { Markdown } from '../dashboards/widgets/markdown';
import './releaseNotes.css';

const longDate = (iso: string) => new Date(`${iso}T00:00:00`).toLocaleDateString(undefined, { year: 'numeric', month: 'long', day: 'numeric' });

/** Release Notes: what changed in each ObserveCraft release, from CHANGELOG.md. */
export function ReleaseNotesPage() {
  const releases = useMemo(() => parseChangelog(changelog), []);
  const [scope, setScope] = useState<string | null>(() => new URLSearchParams(window.location.search).get('area'));
  const [copied, setCopied] = useState<string | null>(null);
  const scopes = useMemo(() => [...new Set(releases.flatMap((r) => r.sections.flatMap((s) => s.items.map((i) => i.scope).filter((x): x is string => !!x))))].sort(), [releases]);
  const shown = releases
    .map((r) => ({ ...r, sections: r.sections.map((s) => ({ ...s, items: s.items.filter((i) => !scope || i.scope === scope) })).filter((s) => s.items.length) }))
    .filter((r) => r.sections.length);
  const copyLink = (version: string) => {
    void navigator.clipboard?.writeText(`${window.location.origin}/release-notes#v${version}`);
    setCopied(version);
    setTimeout(() => setCopied(null), 1500);
  };
  return (
    <div className="rn-page">
      <header className="rn-banner">
        <h1>Release Notes</h1>
        <nav className="rn-areas" aria-label="Product area">
          <button className={`rn-area${scope === null ? ' active' : ''}`} onClick={() => setScope(null)}>
            All Updates
          </button>
          {scopes.map((s) => (
            <button key={s} className={`rn-area${scope === s ? ' active' : ''}`} onClick={() => setScope(s)}>
              {s}
            </button>
          ))}
        </nav>
      </header>
      <main className="rn-releases">
        {shown.length === 0 && <p className="muted">No changes in this area.</p>}
        {shown.map((r) => {
          const areas = [...new Set(r.sections.flatMap((s) => s.items.map((i) => i.scope).filter((x): x is string => !!x)))];
          return (
            <article key={r.version} id={`v${r.version}`} className="rn-release">
              {r.date && <div className="rn-date">{longDate(r.date)}</div>}
              <h2>ObserveCraft {r.version}</h2>
              <div className="rn-meta">
                {areas.map((a) => (
                  <button key={a} className="rn-tag" onClick={() => setScope(a)}>
                    {a}
                  </button>
                ))}
                <button className="link-btn rn-copy" onClick={() => copyLink(r.version)}>
                  {copied === r.version ? 'Copied' : 'Copy URL'} <Link2 size={15} />
                </button>
                {r.url && (
                  <a className="rn-compare" href={r.url} target="_blank" rel="noreferrer">
                    Compare changes
                  </a>
                )}
              </div>
              {r.sections.map((s) => (
                <section key={s.title}>
                  <h3 className={/breaking/i.test(s.title) ? 'rn-breaking' : undefined}>{s.title.replace(/^⚠\s*/, '')}</h3>
                  <ul>
                    {s.items.map((i, n) => (
                      <li key={n}>
                        {i.scope && <strong className="rn-scope">{i.scope}:</strong>}
                        <span className="rn-text">
                          <Markdown source={i.text} />
                        </span>
                      </li>
                    ))}
                  </ul>
                </section>
              ))}
            </article>
          );
        })}
      </main>
    </div>
  );
}
