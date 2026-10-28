import { useEffect, useState, type ReactNode } from 'react';
import { createPortal } from 'react-dom';
import { Link, useNavigate } from 'react-router-dom';
import { Copy, ExternalLink, Pencil, Plus, Trash2, X } from 'lucide-react';
import type { OnCallTeam } from '../../../api/onCall/client';
import type { PortalCatalog, PortalEntity } from '../../../api/portalCatalog/types';
import { useTime } from '../../../components/shell/TimeContext';
import { timeAgo } from '../../../lib/format';
import { fmtLatency } from '../../apm/visuals';
import { LevelBadge } from '../common';
import { useScorecards } from '../queries';
import { definitionYaml, KIND_LABEL, productLink } from './catalogModel';
import { useDeleteDefinition, useEntityDefinition, useEntityDependencies, useEntityDeployments } from './queries';
import { EntityIcon, fmtNumber, fmtPercent } from './visuals';

type Tab = 'overview' | 'dependencies' | 'scorecards' | 'definition';

/** An entity's side panel: overview, dependencies, scorecard levels and its definition. */
export function EntityPanel({ id, data, onCall, onClose, onOpen }: { id: string; data: PortalCatalog; onCall: OnCallTeam[]; onClose: () => void; onOpen: (id: string) => void }) {
  const [tab, setTab] = useState<Tab>('overview');
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => e.key === 'Escape' && onClose();
    globalThis.addEventListener('keydown', onKey);
    return () => globalThis.removeEventListener('keydown', onKey);
  }, [onClose]);
  useEffect(() => setTab('overview'), [id]);
  const e = data.entities.find((x) => x.id === id);
  const navigate = useNavigate();
  const del = useDeleteDefinition();
  const link = e ? productLink(e) : null;
  const tabs: { id: Tab; label: string }[] = [
    { id: 'overview', label: 'Overview' },
    { id: 'dependencies', label: 'Dependencies' },
    ...(e?.kind === 'service' ? [{ id: 'scorecards' as Tab, label: 'Scorecards' }] : []),
    ...(e?.definitionId ? [{ id: 'definition' as Tab, label: 'Definition' }] : []),
  ];

  return createPortal(
    <div className="sc-drawer-backdrop" onMouseDown={(ev) => ev.target === ev.currentTarget && onClose()}>
      <aside className="sc-drawer pc-drawer" role="dialog" aria-label={e ? `${KIND_LABEL[e.kind]} ${e.name}` : 'Entity'}>
        <header className="sc-drawer-head">
          {e && <EntityIcon entity={e} size={22} />}
          <div className="sc-drawer-title">
            <h2>{e ? (e.displayName ?? e.name) : id}</h2>
            {e && (
              <span className="sc-muted">
                {KIND_LABEL[e.kind]}
                {e.kind === 'endpoint' && e.service ? ` of ${e.service}` : ''} · {e.owner ? `Team ${e.owner}` : 'No team'}
                {e.inferred && ' · Inferred'}
              </span>
            )}
          </div>
          <div className="sc-drawer-actions">
            {link && (
              <Link className="btn btn-sm" to={link}>
                <ExternalLink size={13} /> APM
              </Link>
            )}
            {e && e.kind !== 'endpoint' && e.kind !== 'repository' && e.kind !== 'external_provider' && (
              <button
                className="btn btn-sm"
                onClick={() => navigate(e.definitionId ? `/portal/catalog/edit/${e.definitionId}` : `/portal/catalog/new?kind=${e.kind}&name=${encodeURIComponent(e.name)}`)}
              >
                {e.definitionId ? <Pencil size={13} /> : <Plus size={13} />} {e.definitionId ? 'Edit' : 'Add definition'}
              </button>
            )}
            {e?.definitionId && (
              <button
                className="sc-icon-btn"
                title="Delete the definition"
                aria-label="Delete the definition"
                disabled={del.isPending}
                onClick={() => window.confirm(`Delete the definition of ${e.kind}:${e.name}? Telemetry keeps the entity listed.`) && del.mutate(e.definitionId!)}
              >
                <Trash2 size={16} />
              </button>
            )}
            <button className="sc-icon-btn" aria-label="Close" onClick={onClose}>
              <X size={18} />
            </button>
          </div>
        </header>
        {e && (
          <nav className="pc-panel-tabs" role="tablist">
            {tabs.map((t) => (
              <button key={t.id} role="tab" aria-selected={tab === t.id} className={`pc-panel-tab${tab === t.id ? ' active' : ''}`} onClick={() => setTab(t.id)}>
                {t.label}
              </button>
            ))}
          </nav>
        )}
        <div className="sc-drawer-body">
          {!e ? (
            <p className="sc-muted">This entity isn't in the catalog for the selected time range.</p>
          ) : tab === 'overview' ? (
            <Overview e={e} onCall={onCall.find((t) => t.handle === e.owner)} onOpen={onOpen} data={data} />
          ) : tab === 'dependencies' ? (
            <Dependencies e={e} data={data} onOpen={onOpen} />
          ) : tab === 'scorecards' ? (
            <Scorecards service={e.name} />
          ) : (
            <Definition id={e.definitionId!} />
          )}
        </div>
      </aside>
    </div>,
    document.body,
  );
}

function Tile({ label, value, tone }: { label: string; value: ReactNode; tone?: 'alert' }) {
  return (
    <div className={`pc-tile${tone ? ` pc-tile-${tone}` : ''}`}>
      <span className="pc-tile-value">{value}</span>
      <span className="pc-tile-label">{label}</span>
    </div>
  );
}

function Field({ label, children }: { label: string; children: ReactNode }) {
  return (
    <div className="pc-field">
      <span className="pc-field-label">{label}</span>
      <span className="pc-field-value">{children}</span>
    </div>
  );
}

const refId = (ref: string) => {
  const [kind, ...rest] = ref.split(':');
  return `${kind}:${rest.join(':')}`;
};

function Overview({ e, onCall, onOpen, data }: { e: PortalEntity; onCall?: OnCallTeam; onOpen: (id: string) => void; data: PortalCatalog }) {
  const known = new Set(data.entities.map((x) => x.id));
  const isService = e.kind === 'service';
  const { range } = useTime();
  const deploys = useEntityDeployments(isService && e.sources.includes('apm') ? e.name : null, range);
  const refLink = (ref: string) =>
    known.has(refId(ref)) ? (
      <button key={ref} className="pc-chip pc-chip-link" onClick={() => onOpen(refId(ref))}>
        {ref}
      </button>
    ) : (
      <span key={ref} className="pc-chip">
        {ref}
      </span>
    );
  return (
    <>
      {(e.requests !== null || isService) && (
        <section className="pc-panel-section">
          <div className="pc-tiles">
            <Tile label="Requests/s" value={fmtNumber(e.reqPerSec ?? null)} />
            <Tile label="Error rate" value={fmtPercent(e.errorRate ?? null)} tone={(e.errorRate ?? 0) > 0.05 ? 'alert' : undefined} />
            <Tile label="P95 latency" value={e.p95Ms === null || e.p95Ms === undefined ? '—' : fmtLatency(e.p95Ms)} />
            {isService && <Tile label="Monitors in alert" value={`${e.monitorsAlerting} / ${e.monitors}`} tone={e.monitorsAlerting ? 'alert' : undefined} />}
            {isService && <Tile label="SLOs" value={e.slos} />}
            {isService && <Tile label="Open incidents" value={e.openIncidents} tone={e.openIncidents ? 'alert' : undefined} />}
          </div>
        </section>
      )}
      {e.description && <p className="pc-description">{e.description}</p>}
      <section className="pc-panel-section">
        <h3>Ownership</h3>
        <Field label="Team">{e.owner || <span className="sc-muted">No team</span>}</Field>
        <Field label="On-Call">
          {!onCall?.schedules.length ? (
            <span className="sc-muted">{e.owner ? 'The team has no schedule' : '—'}</span>
          ) : (
            <>
              {onCall.onCallNow.length ? onCall.onCallNow.map((u) => u.name).join(', ') : 'Nobody now'}{' '}
              <Link className="sc-link" to="/on-call/schedules">
                ({onCall.schedules.map((s) => s.name).join(', ')})
              </Link>
            </>
          )}
        </Field>
        <Field label="Contacts">
          {e.contacts.length ? (
            <span className="pc-chips">
              {e.contacts.map((c, i) => (
                <span key={i} className="pc-chip" title={c.type}>
                  {c.name ? `${c.name}: ` : ''}
                  {c.contact}
                </span>
              ))}
            </span>
          ) : (
            <span className="sc-muted">None</span>
          )}
        </Field>
        <Field label="Repository">
          {e.repoUrl ? (
            <a className="pc-ext" href={e.repoUrl} target="_blank" rel="noreferrer noopener">
              {e.repoUrl} <ExternalLink size={12} />
            </a>
          ) : (
            <span className="sc-muted">None</span>
          )}
        </Field>
        <Field label="Docs">
          {e.docsUrl ? (
            <a className="pc-ext" href={e.docsUrl} target="_blank" rel="noreferrer noopener">
              {e.docsUrl} <ExternalLink size={12} />
            </a>
          ) : (
            <span className="sc-muted">None</span>
          )}
        </Field>
      </section>
      <section className="pc-panel-section">
        <h3>Details</h3>
        {e.lifecycle && <Field label="Lifecycle">{e.lifecycle}</Field>}
        {e.tier && <Field label="Tier">{e.tier}</Field>}
        {e.entityType && <Field label="Type">{e.entityType}</Field>}
        {e.languages.length > 0 && <Field label="Languages">{e.languages.join(', ')}</Field>}
        {e.systems.length > 0 && <Field label="Systems">{<span className="pc-chips">{e.systems.map((s) => refLink(`system:${s}`))}</span>}</Field>}
        {e.tags.length > 0 && (
          <Field label="Tags">
            <span className="pc-chips">
              {e.tags.map((t) => (
                <span key={t} className="pc-chip">
                  {t}
                </span>
              ))}
            </span>
          </Field>
        )}
        <Field label="Seen in">{e.sources.join(', ')}</Field>
        {e.links.filter((l) => l.type !== 'repo' && l.type !== 'doc').length > 0 && (
          <Field label="Links">
            <span className="pc-links">
              {e.links
                .filter((l) => l.type !== 'repo' && l.type !== 'doc')
                .map((l) => (
                  <a key={l.url} className="pc-ext" href={l.url} target="_blank" rel="noreferrer noopener">
                    {l.name} <span className="sc-muted">({l.type})</span> <ExternalLink size={12} />
                  </a>
                ))}
            </span>
          </Field>
        )}
      </section>
      {isService && (
        <section className="pc-panel-section">
          <h3>Recent deploys</h3>
          {!e.sources.includes('apm') ? (
            <p className="sc-muted">No spans in this time range.</p>
          ) : deploys.isPending ? (
            <p className="sc-muted">Loading…</p>
          ) : !deploys.data?.length ? (
            <p className="sc-muted">No versions seen.</p>
          ) : (
            <table className="pc-table pc-table-compact">
              <thead>
                <tr>
                  <th>Version</th>
                  <th>First seen</th>
                  <th>Requests</th>
                  <th>Error rate</th>
                  <th>P95</th>
                </tr>
              </thead>
              <tbody>
                {deploys.data.slice(0, 6).map((d) => (
                  <tr key={d.version}>
                    <td>
                      <code className="pc-version">{d.version || 'unversioned'}</code>
                    </td>
                    <td>{timeAgo(d.firstSeen)}</td>
                    <td>{d.requests.toLocaleString()}</td>
                    <td>{fmtPercent(d.errorRate)}</td>
                    <td>{fmtLatency(d.p95Ms)}</td>
                  </tr>
                ))}
              </tbody>
            </table>
          )}
          {e.ciSuccessRate !== null && e.ciSuccessRate !== undefined && (
            <p className="sc-muted">
              CI: {fmtPercent(e.ciSuccessRate, 0)} of {e.ciRuns} pipeline runs succeeded in the past 30 days{e.lastCiRun ? `, last ${timeAgo(e.lastCiRun)}` : ''}.
            </p>
          )}
        </section>
      )}
    </>
  );
}

function Dependencies({ e, data, onOpen }: { e: PortalEntity; data: PortalCatalog; onOpen: (id: string) => void }) {
  const { range } = useTime();
  const traced = e.kind === 'service' && e.sources.includes('apm');
  const deps = useEntityDependencies(traced ? e.name : null, range);
  const known = new Set(data.entities.map((x) => x.id));
  // Peers named like the catalog's inferred entities (the APM dependency map names them by peer.service).
  const idOf = (name: string) => [`service:${name}`, `datastore:${name}`, `queue:${name}`, `external_provider:${name}`].find((id) => known.has(id));
  const calledBy = data.entities.filter((x) => (x.kind === 'datastore' || x.kind === 'queue' || x.kind === 'external_provider') && x.related.includes(e.name));
  const list = (title: string, items: { name: string; id?: string; detail?: string }[]) => (
    <section className="pc-panel-section">
      <h3>
        {title} <span className="sc-muted">{items.length}</span>
      </h3>
      {items.length ? (
        <ul className="pc-dep-list">
          {items.map((i) => (
            <li key={i.name}>
              {i.id ? (
                <button className="sc-link" onClick={() => onOpen(i.id!)}>
                  {i.name}
                </button>
              ) : (
                <span>{i.name}</span>
              )}
              {i.detail && <span className="sc-muted">{i.detail}</span>}
            </li>
          ))}
        </ul>
      ) : (
        <p className="sc-muted">None seen in this time range.</p>
      )}
    </section>
  );
  if (e.kind === 'datastore' || e.kind === 'queue' || e.kind === 'external_provider') {
    return list(
      e.kind === 'queue' ? 'Producers and consumers' : 'Called by',
      e.related.map((s) => ({ name: s, id: idOf(s) })),
    );
  }
  if (e.kind === 'system' || e.kind === 'repository' || e.kind === 'api') {
    return list(
      e.kind === 'system' ? 'Components' : e.kind === 'api' ? 'Implemented by' : 'Built into',
      e.related.map((r) => ({ name: r, id: known.has(r) ? r : undefined })),
    );
  }
  if (e.kind === 'endpoint') return list('Service', e.service ? [{ name: e.service, id: idOf(e.service) }] : []);
  const declared = e.dependsOn.map((r) => ({ name: r, id: known.has(r) ? r : undefined, detail: 'declared' }));
  return (
    <>
      {traced && deps.isPending && <p className="sc-muted">Loading dependencies…</p>}
      {list('Calls', [
        ...(deps.data?.downstream ?? []).map((d) => ({ name: d.service, id: idOf(d.service), detail: `${fmtNumber(d.reqPerSec)} req/s · ${fmtPercent(d.errorRate)} errors` })),
        ...calledBy.filter((x) => !(deps.data?.downstream ?? []).some((d) => d.service === x.name)).map((x) => ({ name: x.name, id: x.id, detail: x.technology ?? undefined })),
      ])}
      {list(
        'Called by',
        (deps.data?.upstream ?? []).map((d) => ({ name: d.service, id: idOf(d.service), detail: `${fmtNumber(d.reqPerSec)} req/s` })),
      )}
      {declared.length > 0 && list('Declared dependencies', declared)}
    </>
  );
}

function Scorecards({ service }: { service: string }) {
  const q = useScorecards();
  if (q.isPending) return <p className="sc-muted">Loading scorecards…</p>;
  if (q.isError) return <p className="pc-error">{(q.error as Error).message}</p>;
  if (!q.data.setUp)
    return (
      <p className="sc-muted">
        Scorecards aren't set up yet. <Link to="/portal/scorecards">Set them up</Link> to grade services on ownership, observability and production readiness.
      </p>
    );
  const s = q.data.services.find((x) => x.service === service);
  if (!s) return <p className="sc-muted">This service isn't evaluated by Scorecards: it needs spans in the past 7 days or metadata.</p>;
  return (
    <section className="pc-panel-section">
      <div className="pc-score-head">
        <LevelBadge level={s.level} /> Overall level · {s.pass} pass, {s.fail} fail, {s.skip} skip
      </div>
      <div className="sc-levels-row">
        {q.data.scorecards.map((c) => (
          <div key={c.id} className="sc-level-tile">
            <LevelBadge level={s.levels[c.id] ?? 0} />
            <span>{c.name}</span>
          </div>
        ))}
      </div>
      <Link className="btn btn-sm" to={`/portal/scorecards?view=services&service=${encodeURIComponent(service)}`}>
        Open in Scorecards
      </Link>
    </section>
  );
}

function Definition({ id }: { id: string }) {
  const q = useEntityDefinition(id);
  const [copied, setCopied] = useState(false);
  if (q.isPending) return <p className="sc-muted">Loading…</p>;
  if (q.isError) return <p className="pc-error">{(q.error as Error).message}</p>;
  const yaml = definitionYaml(q.data.definition);
  return (
    <section className="pc-panel-section">
      <p className="sc-muted">
        Written {timeAgo(q.data.updatedAt)} from the {q.data.source === 'ui' ? 'UI' : q.data.source === 'api' ? 'API' : 'import'}. Keep it next to the code as <code>entity.yaml</code> and import it
        again after changes.
      </p>
      <div className="pc-code-wrap">
        <button
          className="btn btn-sm pc-copy"
          onClick={() => {
            void navigator.clipboard?.writeText(yaml).then(() => setCopied(true));
          }}
        >
          <Copy size={13} /> {copied ? 'Copied' : 'Copy'}
        </button>
        <pre className="pc-code">{yaml}</pre>
      </div>
    </section>
  );
}
