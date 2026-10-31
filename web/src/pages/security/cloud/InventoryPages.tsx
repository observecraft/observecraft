import { useState } from 'react';
import { createPortal } from 'react-dom';
import { Link, useSearchParams } from 'react-router-dom';
import { Search, X } from 'lucide-react';
import { Select } from '../../../components/ui/Dropdown';
import { Empty, ErrorState, Loading } from '../../../components/ui/AsyncState';
import { compact, timeAgo } from '../../../lib/format';
import { SeverityBadge } from '../common';
import { InboxPage } from '../InboxPage';
import { INVENTORY_TABS, SubTabs } from './CloudSecurityLayout';
import { AdvisoryBody } from './FindingsPages';
import { useCve, useCves, useInfraPackages, useInfraVulnerabilities, useResourceCategories } from './queries';

const PAGE = 25;

function Chips({ values, max = 2 }: { values: string[]; max?: number }) {
  if (!values.length) return <span className="muted">—</span>;
  return (
    <span className="sec-tags csm-chips">
      {values.slice(0, max).map((v) => (
        <span key={v} className="tag-chip">
          {v}
        </span>
      ))}
      {values.length > max && <span className="tag-chip">+{values.length - max}</span>}
    </span>
  );
}

/** Cloud Security → Inventory → Resources: the catalog by category. */
export function ResourcesPage() {
  const q = useResourceCategories();
  return (
    <div className="sec-page">
      <SubTabs tabs={INVENTORY_TABS} />
      {q.isLoading ? (
        <Loading />
      ) : q.isError ? (
        <ErrorState error={q.error} onRetry={() => q.refetch()} />
      ) : !q.data!.length ? (
        <Empty>
          <b>No resources yet.</b>
          <p className="muted">Hosts, containers and Kubernetes objects show up here once the Agent or the Collector sends them.</p>
        </Empty>
      ) : (
        <>
          <p className="muted">
            <b>{q.data!.length}</b> resource categories · the full list is in <Link to="/infrastructure/catalog">Infrastructure → Catalog</Link>
          </p>
          <table className="mon-table syn-table csm-table">
            <thead>
              <tr>
                <th>RESOURCE CATEGORY</th>
                <th className="num">RESOURCE COUNT</th>
                <th>TYPES</th>
                <th>REGIONS</th>
                <th>ENVIRONMENTS</th>
                <th>TEAMS</th>
                <th>SERVICES</th>
              </tr>
            </thead>
            <tbody>
              {q.data!.map((r) => (
                <tr key={r.category}>
                  <td>
                    <b>{r.category}</b>
                  </td>
                  <td className="num">{compact(r.count)}</td>
                  <td>
                    <Chips values={Object.entries(r.types).map(([k, n]) => `${k.replace(/^kube_/, '')} ${n}`)} max={3} />
                  </td>
                  <td>
                    <Chips values={r.regions} />
                  </td>
                  <td>
                    <Chips values={r.envs} />
                  </td>
                  <td>
                    <Chips values={r.teams} />
                  </td>
                  <td>
                    <Chips values={r.services} />
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        </>
      )}
    </div>
  );
}

/** Cloud Security → Inventory → Infrastructure Packages: what the SBOMs list. */
export function PackagesPage() {
  const q = useInfraPackages();
  const [search, setSearch] = useState('');
  const [group, setGroup] = useState<'version' | 'name'>('version');
  const rows = (q.data ?? []).filter((p) => !search.trim() || `${p.name} ${p.version} ${p.ecosystem}`.toLowerCase().includes(search.trim().toLowerCase()));
  const byName = [...rows.reduce((m, p) => m.set(`${p.ecosystem}:${p.name}`, [...(m.get(`${p.ecosystem}:${p.name}`) ?? []), p]), new Map<string, typeof rows>()).values()];
  return (
    <div className="sec-page">
      <SubTabs tabs={INVENTORY_TABS} />
      <label className="sec-search">
        <Search size={14} />
        <input placeholder="Search for packages" value={search} onChange={(e) => setSearch(e.target.value)} />
      </label>
      <div className="csm-groupby">
        <span>Group by</span>
        <button className={group === 'version' ? 'active' : ''} onClick={() => setGroup('version')}>
          Package and Version
        </button>
        <button className={group === 'name' ? 'active' : ''} onClick={() => setGroup('name')}>
          Package
        </button>
      </div>
      {q.isLoading ? (
        <Loading />
      ) : q.isError ? (
        <ErrorState error={q.error} onRetry={() => q.refetch()} />
      ) : !rows.length ? (
        <Empty>
          <b>{q.data!.length ? 'No package matches.' : 'No packages yet.'}</b>
          <p className="muted">Packages come from the SBOMs the Datadog Agent sends for hosts and container images.</p>
        </Empty>
      ) : (
        <>
          <p className="muted">
            <b>{compact(group === 'name' ? byName.length : rows.length)}</b> packages found
          </p>
          <table className="mon-table syn-table csm-table">
            <thead>
              <tr>
                <th>PACKAGE</th>
                <th>ECOSYSTEM</th>
                <th className="num">PACKAGE RUNNING</th>
                <th className="num">UNIQUE VULNERABILITIES</th>
                <th className="num">RESOURCES</th>
                <th>FIRST DETECTED</th>
              </tr>
            </thead>
            <tbody>
              {(group === 'name'
                ? byName.map((ps) => ({
                    ...ps[0],
                    version: ps.map((p) => p.version).join(', '),
                    resources: ps.reduce((a, p) => a + p.resources, 0),
                    running: ps.reduce((a, p) => a + p.running, 0),
                    vulnerabilities: ps.reduce((a, p) => a + p.vulnerabilities, 0),
                  }))
                : rows
              ).map((p) => (
                <tr key={`${p.ecosystem}:${p.name}:${p.version}`}>
                  <td className="mono">
                    <b>{p.name}</b> {p.version}
                  </td>
                  <td>{p.ecosystem}</td>
                  <td className="num">{p.running}</td>
                  <td className="num">
                    {p.vulnerabilities ? (
                      <Link to={`/security/cloud/vulnerabilities?q=${encodeURIComponent(`@package:${p.name}`)}`}>
                        {p.maxSeverity && <i className={`sec-dot sev-${p.maxSeverity}`} />} {p.vulnerabilities}
                      </Link>
                    ) : (
                      0
                    )}
                  </td>
                  <td className="num">{p.resources}</td>
                  <td className="muted">{timeAgo(p.firstSeen)}</td>
                </tr>
              ))}
            </tbody>
          </table>
        </>
      )}
    </div>
  );
}

/** Cloud Security → Inventory → CVE Explorer: every advisory, with whether it affects you. */
export function CveExplorerPage() {
  const [params, setParams] = useSearchParams();
  const [draft, setDraft] = useState(params.get('q') ?? '');
  const query = params.get('q') ?? '';
  const severity = params.get('severity') ?? '';
  const impact = params.get('impact') ?? '';
  const exploited = params.get('exploited') === 'true';
  const offset = Number(params.get('offset') ?? 0);
  const open = params.get('cve');
  const set = (k: string, v: string | null) =>
    setParams(
      (p) => {
        const n = new URLSearchParams(p);
        if (v) n.set(k, v);
        else n.delete(k);
        if (k !== 'offset' && k !== 'cve') n.delete('offset');
        return n;
      },
      { replace: true },
    );
  const q = useCves({ query, severity, impact, exploited, offset, limit: PAGE });
  const vulns = useInfraVulnerabilities('all');
  const total = q.data?.total ?? 0;
  return (
    <div className="sec-page">
      <SubTabs tabs={INVENTORY_TABS} />
      <div className="sec-filters">
        <form
          className="sec-search"
          onSubmit={(e) => {
            e.preventDefault();
            set('q', draft.trim() || null);
          }}
        >
          <Search size={14} />
          <input placeholder="Search by CVE ID, advisory ID, or package name" value={draft} onChange={(e) => setDraft(e.target.value)} />
        </form>
        <Select
          label="Severity"
          value={severity}
          onChange={(v) => set('severity', v || null)}
          options={[{ value: '', label: 'All' }, ...['critical', 'high', 'medium', 'low'].map((s) => ({ value: s, label: s[0].toUpperCase() + s.slice(1) }))]}
        />
        <Select
          label="Impact"
          value={impact}
          onChange={(v) => set('impact', v || null)}
          options={[
            { value: '', label: 'All' },
            { value: 'impacted', label: 'Impacted' },
            { value: 'not_impacted', label: 'Not impacted' },
          ]}
        />
        <Select
          label="CISA Known Exploit"
          value={exploited ? 'true' : ''}
          onChange={(v) => set('exploited', v || null)}
          options={[
            { value: '', label: 'All' },
            { value: 'true', label: 'Known exploited' },
          ]}
        />
      </div>
      {q.isLoading ? (
        <Loading />
      ) : q.isError ? (
        <ErrorState error={q.error} onRetry={() => q.refetch()} />
      ) : !q.data!.rows.length ? (
        <Empty>
          <b>No advisories match.</b>
          <p className="muted">Advisories are looked up in OSV.dev for the packages and libraries you use.</p>
        </Empty>
      ) : (
        <>
          <div className="csm-pager">
            <span>
              <b>
                {offset + 1}–{Math.min(offset + PAGE, total)}
              </b>{' '}
              of <b>{compact(total)}</b> advisories
            </span>
            <span className="syn-grow" />
            <button className="btn btn-sm" disabled={offset === 0} onClick={() => set('offset', String(Math.max(0, offset - PAGE)))}>
              Previous
            </button>
            <button className="btn btn-sm" disabled={offset + PAGE >= total} onClick={() => set('offset', String(offset + PAGE))}>
              Next
            </button>
          </div>
          <table className="mon-table syn-table csm-table csm-cves">
            <thead>
              <tr>
                <th>IDENTIFICATION</th>
                <th>SUMMARY</th>
                <th>BASE SEVERITY</th>
                <th>IMPACT</th>
                <th>PUBLISHED DATE</th>
              </tr>
            </thead>
            <tbody>
              {q.data!.rows.map((r) => {
                const impacted = r.impactedResources + r.impactedLibraries > 0;
                return (
                  <tr key={r.id} className={open === r.id ? 'selected' : ''} onClick={() => set('cve', r.id)}>
                    <td>
                      <span className="tag-chip mono">{r.cve ?? r.id}</span>
                      {r.exploited && <span className="tag-chip sec-ransom">CISA KEV</span>}
                    </td>
                    <td>
                      <b>{r.summary}</b>
                      <div className="sec-tags">
                        <span className="muted">packages:</span>
                        {r.packages.slice(0, 3).map((p) => (
                          <span key={p} className="tag-chip mono">
                            {p}
                          </span>
                        ))}
                      </div>
                    </td>
                    <td className="nowrap">
                      <span className={`csm-score sev-text-${r.severity}`}>
                        {r.severity.toUpperCase()} {r.score?.toFixed(1)}
                      </span>
                    </td>
                    <td className={impacted ? 'csm-impacted' : 'muted'}>{impacted ? 'Impacted' : 'Not impacted'}</td>
                    <td className="nowrap muted">
                      {r.published ? new Date(r.published).toLocaleString(undefined, { year: 'numeric', month: 'short', day: 'numeric', hour: '2-digit', minute: '2-digit' }) : '—'}
                    </td>
                  </tr>
                );
              })}
            </tbody>
          </table>
        </>
      )}
      {open &&
        createPortal(
          <CveDrawer id={open} resources={(vulns.data ?? []).filter((v) => v.advisoryId === open)} row={q.data?.rows.find((r) => r.id === open)} onClose={() => set('cve', null)} />,
          document.body,
        )}
    </div>
  );
}

function CveDrawer({
  id,
  row,
  resources,
  onClose,
}: {
  id: string;
  row?: { cve?: string | null; impactedLibraries: number; impactedResources: number };
  resources: { resourceId: string; resourceName: string; resourceKind: string; running: boolean }[];
  onClose: () => void;
}) {
  const a = useCve(id);
  const uniq = [...new Map(resources.map((r) => [r.resourceId, r])).values()];
  return (
    <div className="sec-panel-backdrop" onMouseDown={(e) => e.target === e.currentTarget && onClose()}>
      <aside className="sec-panel csm-drawer" role="dialog" aria-label={id}>
        <header className="sec-panel-head">
          <span className="muted mono">
            {row?.cve && <>CVE: {row.cve} · </>}Advisory ID: {id}
          </span>
          <span className="syn-grow" />
          <button className="btn btn-ghost" onClick={onClose} aria-label="Close">
            <X size={16} />
          </button>
        </header>
        <div className="csm-drawer-body">
          {a.isLoading ? (
            <Loading />
          ) : a.isError ? (
            <ErrorState error={a.error} />
          ) : (
            <>
              <h2 className="csm-drawer-title">{a.data!.summary}</h2>
              <p>
                <SeverityBadge severity={a.data!.severity} /> {a.data!.score != null && <b>{a.data!.score}</b>}{' '}
                <span className="muted">{a.data!.published ? `Published ${timeAgo(a.data!.published)}` : ''}</span>
              </p>
              <h4 className="csm-impacted">Impacted resources</h4>
              <div className="csm-impact">
                <div className={row?.impactedLibraries ? 'bad' : 'ok'}>
                  <b>{row?.impactedLibraries ?? 0}</b> libraries of your services (Code Security)
                </div>
                <div className={uniq.length ? 'bad' : 'ok'}>
                  <b>{uniq.length}</b> hosts and images
                </div>
                <div className={uniq.some((r) => r.running) ? 'bad' : 'ok'}>
                  <b>{uniq.filter((r) => r.running).length}</b> running
                </div>
              </div>
              {uniq.length > 0 && (
                <ul className="csm-list">
                  {uniq.map((r) => (
                    <li key={r.resourceId}>
                      {r.resourceName} <span className="muted">({r.resourceKind.replace('_', ' ')})</span>
                    </li>
                  ))}
                </ul>
              )}
              <AdvisoryBody a={a.data!} />
            </>
          )}
        </div>
      </aside>
    </div>
  );
}

/** Cloud Security → Inbox: the Security Inbox's cloud findings. */
export function CloudInboxPage() {
  return <InboxPage types={['Misconfiguration', 'Infrastructure Vulnerability']} />;
}
