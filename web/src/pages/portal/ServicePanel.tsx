import { useEffect, useState } from 'react';
import { createPortal } from 'react-dom';
import { Link } from 'react-router-dom';
import { ExternalLink, Pencil, X } from 'lucide-react';
import type { ScorecardsOverview } from '../../api/scorecards/client';
import { serviceOutcomes } from '../../api/scorecards/logic';
import { fmtPct, LevelBadge, StateIcon } from './common';
import { MetadataModal } from './MetadataModal';

/** A service's scorecard: its level per scorecard, every outcome with its reason, and its metadata. */
export function ServicePanel({ data, service, onClose, onRule }: { data: ScorecardsOverview; service: string; onClose: () => void; onRule: (id: string) => void }) {
  const [editing, setEditing] = useState(false);
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => e.key === 'Escape' && !editing && onClose();
    globalThis.addEventListener('keydown', onKey);
    return () => globalThis.removeEventListener('keydown', onKey);
  }, [onClose, editing]);
  const s = data.services.find((x) => x.service === service);
  const rows = serviceOutcomes(service, data.rules, data.outcomes);
  // What to fix next: the failing rules of the lowest level that fails.
  const nextLevel = s ? s.level + 1 : 1;
  const blocking = rows.filter((r) => r.outcome.state === 'fail' && r.rule.level === nextLevel);

  return createPortal(
    <div className="sc-drawer-backdrop" onMouseDown={(e) => e.target === e.currentTarget && onClose()}>
      <aside className="sc-drawer" role="dialog" aria-label={`Service ${service}`}>
        <header className="sc-drawer-head">
          <LevelBadge level={s?.level ?? 0} />
          <div className="sc-drawer-title">
            <h2>{service}</h2>
            <span className="sc-muted">
              {s?.team ? `Team ${s.team}` : 'No team'} · score {fmtPct(s?.score)}
            </span>
          </div>
          <div className="sc-drawer-actions">
            {s?.instrumented && (
              <Link className="btn btn-sm" to={`/apm/services/${encodeURIComponent(service)}`}>
                Service page
              </Link>
            )}
            <button className="sc-icon-btn" aria-label="Close" onClick={onClose}>
              <X size={18} />
            </button>
          </div>
        </header>
        <div className="sc-drawer-body">
          {!s ? (
            <p className="sc-muted">This service isn't listed: it has no spans in the past 7 days and no metadata.</p>
          ) : (
            <>
              <section className="sc-panel-section">
                <div className="sc-levels-row">
                  {data.scorecards.map((c) => (
                    <div key={c.id} className="sc-level-tile">
                      <LevelBadge level={s.levels[c.id] ?? 0} />
                      <span>{c.name}</span>
                    </div>
                  ))}
                </div>
                {blocking.length > 0 && (
                  <p className="sc-note">
                    To reach level {nextLevel}: {blocking.map((b) => b.rule.name).join(', ')}.
                  </p>
                )}
              </section>

              <section className="sc-panel-section">
                <div className="sc-panel-head">
                  <h3>Service metadata</h3>
                  <button className="btn btn-sm" onClick={() => setEditing(true)}>
                    <Pencil size={13} /> Edit
                  </button>
                </div>
                <dl className="sc-meta">
                  <dt>Team</dt>
                  <dd>{s.team || <span className="sc-muted">Not set</span>}</dd>
                  <dt>Contacts</dt>
                  <dd>
                    {s.contacts.length ? (
                      s.contacts.map((c, i) => (
                        <span key={i} className="sc-contact">
                          <span className="sc-muted">{c.name || c.type}:</span>{' '}
                          {c.type === 'link' ? (
                            <a href={c.value} target="_blank" rel="noreferrer noopener">
                              {c.value}
                            </a>
                          ) : c.type === 'email' ? (
                            <a href={`mailto:${c.value}`}>{c.value}</a>
                          ) : (
                            c.value
                          )}
                        </span>
                      ))
                    ) : (
                      <span className="sc-muted">None</span>
                    )}
                  </dd>
                  <dt>Code repository</dt>
                  <dd>{s.repoUrl ? <ExtLink href={s.repoUrl} /> : <span className="sc-muted">Not set</span>}</dd>
                  <dt>Docs</dt>
                  <dd>{s.docsUrl ? <ExtLink href={s.docsUrl} /> : <span className="sc-muted">Not set</span>}</dd>
                </dl>
              </section>

              {data.scorecards.map((c) => {
                const mine = rows.filter((r) => r.rule.scorecardId === c.id).sort((a, b) => a.rule.level - b.rule.level || a.rule.name.localeCompare(b.rule.name));
                if (!mine.length) return null;
                return (
                  <section key={c.id} className="sc-panel-section">
                    <div className="sc-panel-head">
                      <h3>{c.name}</h3>
                      <LevelBadge level={s.levels[c.id] ?? 0} />
                    </div>
                    <ul className="sc-outcome-list">
                      {mine.map(({ rule, outcome }) => (
                        <li key={rule.id}>
                          <StateIcon state={outcome.state} />
                          <button className="sc-link" onClick={() => onRule(rule.id)}>
                            {rule.name}
                          </button>
                          <LevelBadge level={rule.level} />
                          <span className="sc-outcome-reason">{outcome.reason}</span>
                        </li>
                      ))}
                    </ul>
                  </section>
                );
              })}
            </>
          )}
        </div>
      </aside>
      {editing && s && <MetadataModal service={s} onClose={() => setEditing(false)} />}
    </div>,
    document.body,
  );
}

function ExtLink({ href }: { href: string }) {
  return (
    <a href={href} target="_blank" rel="noreferrer noopener" className="sc-ext">
      {href} <ExternalLink size={12} />
    </a>
  );
}
