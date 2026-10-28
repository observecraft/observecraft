import { useMemo, useState, type ReactNode } from 'react';
import { Link, useSearchParams } from 'react-router-dom';
import { ExternalLink, Info, Mail, MessageSquare, MoreVertical, Pin, PinOff, Plus, Search, Server, Settings, Upload, Users, X } from 'lucide-react';
import type { OnCallTeam } from '../../../api/onCall/client';
import type { PortalCatalog, PortalEntity } from '../../../api/portalCatalog/types';
import { ErrorState, Loading } from '../../../components/ui/AsyncState';
import { Menu, Select } from '../../../components/ui/Dropdown';
import { useTime } from '../../../components/shell/TimeContext';
import { timeAgo } from '../../../lib/format';
import { fmtLatency } from '../../apm/visuals';
import { columnsFor, COLUMN_LABEL, filterEntities, KIND_LABEL, navCounts, productLink, SOFTWARE_SECTIONS, VIEWS, type Column, type Section, type ViewId } from './catalogModel';
import { CatalogWelcome } from './CatalogWelcome';
import { EntityPanel } from './EntityPanel';
import { ImportModal } from './ImportModal';
import { useInfrastructureCount, useOnCallTeams, usePortalCatalog } from './queries';
import { EntityIcon, fmtNumber, fmtPercent, SECTION_ICON, VIEW_ICON } from './visuals';
import './catalog.css';

const BANNER_KEY = 'oc.portal.catalog.banner-dismissed';
const PINNED_TEAM_KEY = 'oc.portal.catalog.pinned-team';
const VIEWS_KEY = 'oc.portal.catalog.saved-views';

interface SavedView {
  name: string;
  params: string;
}

const read = <T,>(key: string, fallback: T): T => {
  try {
    const v = localStorage.getItem(key);
    return v === null ? fallback : (JSON.parse(v) as T);
  } catch {
    return fallback;
  }
};
const write = (key: string, value: unknown) => {
  try {
    if (value === null) localStorage.removeItem(key);
    else localStorage.setItem(key, JSON.stringify(value));
  } catch {
    /* storage unavailable */
  }
};

/** Developer Portal → Catalog: every entity from telemetry and definitions, by section and view. */
export function SoftwareCatalogPage() {
  const { range } = useTime();
  const [params, setParams] = useSearchParams();
  const env = params.get('env');
  const q = usePortalCatalog(range, env);
  const infra = useInfrastructureCount(range);
  const onCall = useOnCallTeams();
  const [importing, setImporting] = useState(false);

  if (q.isPending) return <Loading label="Loading the catalog…" />;
  if (q.isError) return <ErrorState error={q.error} onRetry={() => q.refetch()} />;
  const data = q.data;
  const empty = !data.entities.some((e) => e.kind !== 'endpoint' && e.kind !== 'repository');
  return (
    <div className="pc-page">
      <ViewsBar params={params} setParams={setParams} onImport={() => setImporting(true)} />
      <div className="pc-body">
        <CatalogNav data={data} params={params} setParams={setParams} infrastructure={infra.data} />
        <main className="pc-main">
          {empty ? (
            <CatalogWelcome onImport={() => setImporting(true)} />
          ) : params.get('section') === 'teams' ? (
            <TeamsList data={data} onCall={onCall.data ?? []} onTeam={(t) => setParams({ team: t })} />
          ) : (
            <EntitiesView data={data} params={params} setParams={setParams} onCall={onCall.data ?? []} />
          )}
        </main>
      </div>
      {params.get('entity') && (
        <EntityPanel
          id={params.get('entity')!}
          data={data}
          onCall={onCall.data ?? []}
          onClose={() => setParams((p) => (p.delete('entity'), new URLSearchParams(p)))}
          onOpen={(id) => setParams((p) => (p.set('entity', id), new URLSearchParams(p)))}
        />
      )}
      {importing && <ImportModal onClose={() => setImporting(false)} />}
    </div>
  );
}

type SetParams = ReturnType<typeof useSearchParams>[1];

function ViewsBar({ params, setParams, onImport }: { params: URLSearchParams; setParams: SetParams; onImport: () => void }) {
  const [views, setViews] = useState<SavedView[]>(() => read<SavedView[]>(VIEWS_KEY, []));
  const current = views.find((v) => v.params === withoutPanel(params));
  const save = (next: SavedView[]) => (setViews(next), write(VIEWS_KEY, next));
  return (
    <div className="pc-viewsbar">
      <span className="pc-viewsbar-label">Views</span>
      <Select
        value={current?.name ?? ''}
        options={[{ value: '', label: 'My View' }, ...views.map((v) => ({ value: v.name, label: v.name }))]}
        onChange={(name) => setParams(new URLSearchParams(views.find((v) => v.name === name)?.params ?? ''))}
        triggerClassName="pc-view-trigger"
      />
      <Menu
        trigger={<MoreVertical size={15} aria-label="View actions" />}
        triggerClassName="pc-icon-btn"
        align="left"
        items={[
          {
            label: 'Save as a new view',
            onClick: () => {
              const name = window.prompt('Name of the view')?.trim();
              if (name) save([...views.filter((v) => v.name !== name), { name, params: withoutPanel(params) }]);
            },
          },
          ...(current ? [{ label: `Delete “${current.name}”`, onClick: () => save(views.filter((v) => v !== current)) }] : []),
          { label: 'Reset filters', onClick: () => setParams(new URLSearchParams()) },
        ]}
      />
      <div className="pc-viewsbar-actions">
        <button className="btn btn-sm" onClick={onImport}>
          <Upload size={14} /> Import
        </button>
        <Link className="btn btn-sm btn-primary" to="/portal/catalog/new">
          <Plus size={14} /> Add Entity
        </Link>
      </div>
    </div>
  );
}

const withoutPanel = (p: URLSearchParams) => {
  const next = new URLSearchParams(p);
  next.delete('entity');
  next.sort();
  return next.toString();
};

function CatalogNav({ data, params, setParams, infrastructure }: { data: PortalCatalog; params: URLSearchParams; setParams: SetParams; infrastructure?: number }) {
  const counts = navCounts(data.entities, data.myTeams);
  const section = params.get('section') ?? 'all';
  const go = (s: string) =>
    setParams((p) => {
      const next = new URLSearchParams(p);
      if (s === 'all') next.delete('section');
      else next.set('section', s);
      next.delete('entity');
      return next;
    });
  const item = (id: string, label: string, icon: ReactNode, count: number | undefined) => (
    <button key={id} className={`pc-nav-item${section === id ? ' active' : ''}`} onClick={() => go(id)}>
      {icon}
      <span className="pc-nav-label">{label}</span>
      <span className="pc-nav-count">{count ?? 0}</span>
    </button>
  );
  return (
    <aside className="pc-nav" aria-label="Catalog sections">
      {item('teams', 'Teams', <Users size={15} />, data.teams.length)}
      <Link className="pc-nav-item" to="/infrastructure/catalog">
        <Server size={15} />
        <span className="pc-nav-label">Infrastructure</span>
        <span className="pc-nav-count">{infrastructure ?? '…'}</span>
      </Link>
      <div className="pc-nav-group">
        <button className={`pc-nav-group-title${section === 'all' ? ' active' : ''}`} onClick={() => go('all')}>
          Software <span className="pc-nav-count">{counts.software}</span>
        </button>
        <Link className="pc-icon-btn" to="/portal/settings" title="Catalog settings" aria-label="Catalog settings">
          <Settings size={14} />
        </Link>
      </div>
      {SOFTWARE_SECTIONS.map((s) => {
        const Icon = SECTION_ICON[s.id];
        return item(s.id, s.label, <Icon size={15} />, counts[s.id]);
      })}
    </aside>
  );
}

function EntitiesView({ data, params, setParams, onCall }: { data: PortalCatalog; params: URLSearchParams; setParams: SetParams; onCall: OnCallTeam[] }) {
  const [banner, setBanner] = useState(() => !read(BANNER_KEY, false));
  const [pinned, setPinned] = useState<string | null>(() => read<string | null>(PINNED_TEAM_KEY, null));
  const [lifecycle, setLifecycle] = useState<string | null>(null);
  const [tier, setTier] = useState<string | null>(null);
  const section = (params.get('section') ?? 'all') as Section;
  const view = (VIEWS.find((v) => v.id === params.get('view'))?.id ?? 'ownership') as ViewId;
  const search = params.get('q') ?? '';
  const team = params.get('team') ?? pinned ?? '';
  const set = (k: string, v: string | null) =>
    setParams((p) => {
      const next = new URLSearchParams(p);
      if (v) next.set(k, v);
      else next.delete(k);
      return next;
    });
  const rows = useMemo(() => {
    const base = filterEntities(data.entities, { section, search, teams: team ? [team] : [], myTeams: data.myTeams });
    return base.filter((e) => (!lifecycle || e.lifecycle === lifecycle) && (!tier || e.tier === tier));
  }, [data, section, search, team, lifecycle, tier]);
  const lifecycles = [...new Set(data.entities.map((e) => e.lifecycle).filter((x): x is string => !!x))].sort();
  const tiers = [...new Set(data.entities.map((e) => e.tier).filter((x): x is string => !!x))].sort();
  const columns = columnsFor(view);
  const onCallByTeam = new Map(onCall.map((t) => [t.handle, t]));
  const title = section === 'all' ? 'All software' : (SOFTWARE_SECTIONS.find((s) => s.id === section)?.label ?? 'Software');

  return (
    <>
      {banner && (
        <div className="pc-banner" role="note">
          <Info size={16} />
          <span>
            Entities come from your telemetry (APM services, the datastores, queues and providers they call, services named in logs and metrics) and from the definitions you add or import. Definitions
            use the entity definition format <code>apiVersion: v3</code>.
          </span>
          <button className="pc-icon-btn" aria-label="Dismiss" onClick={() => (setBanner(false), write(BANNER_KEY, true))}>
            <X size={15} />
          </button>
        </div>
      )}
      <label className="pc-search">
        <Search size={15} />
        <input value={search} onChange={(e) => set('q', e.target.value || null)} placeholder="Search by name or tags" aria-label="Search by name or tags" />
      </label>
      <div className="pc-filters">
        <div className="pc-filter-box">
          <Select
            label="Teams"
            value={team}
            options={[{ value: '', label: 'All teams' }, ...data.teams.map((t) => ({ value: t.handle, label: t.name, hint: t.entities || undefined }))]}
            onChange={(v) => set('team', v || null)}
          />
          <button
            className={`pc-icon-btn${pinned && pinned === team ? ' pc-pinned' : ''}`}
            title={pinned && pinned === team ? 'Unpin this team' : 'Pin this team as your default'}
            aria-label={pinned && pinned === team ? 'Unpin team' : 'Pin team'}
            disabled={!team}
            onClick={() => {
              const next = pinned === team ? null : team;
              setPinned(next);
              write(PINNED_TEAM_KEY, next);
            }}
          >
            {pinned && pinned === team ? <PinOff size={14} /> : <Pin size={14} />}
          </button>
        </div>
        <Select
          label="Env"
          value={params.get('env') ?? ''}
          options={[{ value: '', label: 'All environments' }, ...data.envs.map((e) => ({ value: e, label: e }))]}
          onChange={(v) => set('env', v || null)}
        />
        {lifecycle !== null && <FilterChip label="Lifecycle" value={lifecycle} options={lifecycles} onChange={setLifecycle} onRemove={() => setLifecycle(null)} />}
        {tier !== null && <FilterChip label="Tier" value={tier} options={tiers} onChange={setTier} onRemove={() => setTier(null)} />}
        <Menu
          trigger={
            <>
              <Plus size={14} /> Filter
            </>
          }
          triggerClassName="btn btn-sm pc-add-filter"
          align="left"
          items={[
            { label: 'Lifecycle', onClick: () => setLifecycle(lifecycles[0] ?? ''), disabled: lifecycle !== null },
            { label: 'Tier', onClick: () => setTier(tiers[0] ?? ''), disabled: tier !== null },
          ]}
        />
      </div>
      <div className="pc-view-tabs" role="tablist">
        {VIEWS.map((v) => {
          const Icon = VIEW_ICON[v.id];
          return (
            <button key={v.id} role="tab" aria-selected={v.id === view} className={`pc-view-tab${v.id === view ? ' active' : ''}`} onClick={() => set('view', v.id === 'ownership' ? null : v.id)}>
              <Icon size={15} /> {v.label}
            </button>
          );
        })}
      </div>
      <div className="pc-table-head">
        <span className="pc-table-title">
          {title} <span className="sc-muted">{rows.length}</span>
        </span>
        {(view === 'security' || view === 'costs') && (
          <span className="sc-muted">{view === 'security' ? 'No security data source is connected yet.' : 'No cloud cost data source is connected yet.'}</span>
        )}
      </div>
      <div className="pc-table-wrap">
        <table className="pc-table">
          <thead>
            <tr>
              {columns.map((c) => (
                <th key={c}>{COLUMN_LABEL[c]}</th>
              ))}
            </tr>
          </thead>
          <tbody>
            {rows.slice(0, 500).map((e) => (
              <tr key={e.id} className="pc-row" onClick={() => set('entity', e.id)}>
                {columns.map((c) => (
                  <td key={c}>
                    <Cell column={c} e={e} onCall={onCallByTeam.get(e.owner)} />
                  </td>
                ))}
              </tr>
            ))}
            {!rows.length && (
              <tr>
                <td colSpan={columns.length} className="pc-empty-row">
                  No entities match.
                </td>
              </tr>
            )}
          </tbody>
        </table>
        {rows.length > 500 && <p className="sc-muted pc-more">Showing 500 of {rows.length}; narrow the search to see the rest.</p>}
      </div>
    </>
  );
}

function FilterChip({ label, value, options, onChange, onRemove }: { label: string; value: string; options: string[]; onChange: (v: string) => void; onRemove: () => void }) {
  return (
    <div className="pc-filter-box">
      <Select label={label} value={value} options={options.length ? options.map((o) => ({ value: o, label: o })) : [{ value: '', label: 'None defined' }]} onChange={onChange} />
      <button className="pc-icon-btn" aria-label={`Remove ${label} filter`} onClick={onRemove}>
        <X size={14} />
      </button>
    </div>
  );
}

const dash = <span className="sc-muted">—</span>;

function Cell({ column, e, onCall }: { column: Column; e: PortalEntity; onCall?: OnCallTeam }) {
  const isService = e.kind === 'service';
  switch (column) {
    case 'name': {
      const link = productLink(e);
      return (
        <span className="pc-name">
          <EntityIcon entity={e} />
          <span className="pc-name-text" title={e.kind === 'endpoint' ? `${e.service} · ${e.name}` : e.name}>
            {e.displayName ?? e.name}
          </span>
          {e.kind === 'endpoint' && <span className="sc-muted pc-name-sub">{e.service}</span>}
          {e.inferred && <span className="pc-pill">Inferred</span>}
          {link && (
            <Link to={link} className="pc-row-link" onClick={(ev) => ev.stopPropagation()} title="Open in APM" aria-label="Open in APM">
              <ExternalLink size={13} />
            </Link>
          )}
        </span>
      );
    }
    case 'type':
      return (
        <span>
          {KIND_LABEL[e.kind]}
          {e.entityType ? <span className="sc-muted"> · {e.entityType}</span> : null}
        </span>
      );
    case 'team':
      return e.owner ? <span className="pc-team">{e.owner}</span> : <span className="sc-muted">No team</span>;
    case 'onCall':
      if (!onCall || !onCall.schedules.length) return dash;
      return onCall.onCallNow.length ? (
        <span title={onCall.schedules.map((s) => s.name).join(', ')}>{onCall.onCallNow.map((u) => u.name).join(', ')}</span>
      ) : (
        <span className="sc-muted">Nobody now</span>
      );
    case 'contacts':
      return e.contacts.length ? (
        <span className="pc-contacts">
          {e.contacts.slice(0, 3).map((c, i) => (
            <span key={i} className="pc-contact" title={`${c.name ? `${c.name}: ` : ''}${c.contact}`}>
              {c.type === 'email' ? <Mail size={13} /> : <MessageSquare size={13} />} {c.contact}
            </span>
          ))}
        </span>
      ) : (
        dash
      );
    case 'repo':
      return e.repoUrl ? <ExtLink url={e.repoUrl} label={e.repoUrl.replace(/^https?:\/\/[^/]+\//, '')} /> : dash;
    case 'docs':
      return e.docsUrl ? <ExtLink url={e.docsUrl} label="Docs" /> : dash;
    case 'slos':
      return isService ? <span>{e.slos}</span> : dash;
    case 'monitors':
      return isService ? <span className={e.monitorsAlerting ? 'pc-alert' : undefined}>{e.monitors ? `${e.monitorsAlerting} / ${e.monitors}` : '0'}</span> : dash;
    case 'incidents':
      return isService ? <span className={e.openIncidents ? 'pc-alert' : undefined}>{e.openIncidents ? `${e.incidents} (${e.openIncidents} open)` : e.incidents}</span> : dash;
    case 'errors':
      return e.errorIssues === null || e.errorIssues === undefined ? dash : <span className={e.errorIssues ? 'pc-alert' : undefined}>{e.errorIssues}</span>;
    case 'requests':
      return e.reqPerSec === null || e.reqPerSec === undefined ? e.requests ? <span>{e.requests.toLocaleString()} req</span> : dash : <span>{fmtNumber(e.reqPerSec)}</span>;
    case 'errorRate':
      return e.errorRate === null || e.errorRate === undefined ? dash : <span className={e.errorRate > 0.05 ? 'pc-alert' : undefined}>{fmtPercent(e.errorRate)}</span>;
    case 'p95':
      return e.p95Ms === null || e.p95Ms === undefined ? dash : <span>{fmtLatency(e.p95Ms)}</span>;
    case 'vulnerabilities':
    case 'signals':
    case 'cost':
    case 'costChange':
      return (
        <span className="sc-muted" title="Not available: no data source">
          —
        </span>
      );
    case 'version':
      return e.version ? <code className="pc-version">{e.version}</code> : dash;
    case 'lastDeploy':
      return e.lastDeploy ? <span title={new Date(e.lastDeploy).toLocaleString()}>{timeAgo(e.lastDeploy)}</span> : dash;
    case 'ciSuccess':
      return e.ciSuccessRate === null || e.ciSuccessRate === undefined ? dash : <span title={`${e.ciRuns} runs in 30 days`}>{fmtPercent(e.ciSuccessRate, 0)}</span>;
    case 'lastCiRun':
      return e.lastCiRun ? <span>{timeAgo(e.lastCiRun)}</span> : dash;
  }
}

function ExtLink({ url, label }: { url: string; label: string }) {
  return (
    <a className="pc-ext" href={url} target="_blank" rel="noreferrer noopener" onClick={(e) => e.stopPropagation()}>
      {label} <ExternalLink size={12} />
    </a>
  );
}

function TeamsList({ data, onCall, onTeam }: { data: PortalCatalog; onCall: OnCallTeam[]; onTeam: (handle: string) => void }) {
  const byTeam = new Map(onCall.map((t) => [t.handle, t]));
  return (
    <>
      <div className="pc-table-head">
        <span className="pc-table-title">
          Teams <span className="sc-muted">{data.teams.length}</span>
        </span>
        <Link to="/teams" className="btn btn-sm">
          Manage teams
        </Link>
      </div>
      <div className="pc-table-wrap">
        <table className="pc-table">
          <thead>
            <tr>
              <th>Team</th>
              <th>Members</th>
              <th>Entities</th>
              <th>On-Call now</th>
            </tr>
          </thead>
          <tbody>
            {data.teams.map((t) => {
              const oc = byTeam.get(t.handle);
              return (
                <tr key={t.handle} className="pc-row" onClick={() => onTeam(t.handle)}>
                  <td>
                    <span className="pc-name">
                      <Users size={15} className="pc-kind-icon" />
                      <span className="pc-name-text">{t.name}</span>
                      <span className="sc-muted">@{t.handle}</span>
                    </span>
                  </td>
                  <td>{t.members}</td>
                  <td>{t.entities}</td>
                  <td>{oc?.onCallNow.length ? oc.onCallNow.map((u) => u.name).join(', ') : <span className="sc-muted">{oc?.schedules.length ? 'Nobody now' : 'No schedule'}</span>}</td>
                </tr>
              );
            })}
            {!data.teams.length && (
              <tr>
                <td colSpan={4} className="pc-empty-row">
                  No teams yet.
                </td>
              </tr>
            )}
          </tbody>
        </table>
      </div>
    </>
  );
}
