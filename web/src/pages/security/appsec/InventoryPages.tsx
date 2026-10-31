import { useState, type ReactNode } from 'react';
import { Link, useSearchParams } from 'react-router-dom';
import { Code2, Download, HelpCircle, ListTree, Search, Shield, Star } from 'lucide-react';
import type { ApiEndpointView, AppsecService } from '../../../api/appsec/client';
import { FacetPanel } from '../../../components/facets/FacetPanel';
import { Empty, ErrorState, Loading } from '../../../components/ui/AsyncState';
import { compact, timeAgo } from '../../../lib/format';
import { formatQuery, parseQuery, type Term } from '../../../lib/searchQuery';
import { LANGUAGE_LABEL, useStars } from '../../apm/visuals';
import { AapTabs, INVENTORY_TABS } from './AppsecLayout';
import { ActivitySpark, downloadCsv, ExplorerToolbar, localFacets, MethodPath, ServiceChip } from './common';
import { useApiEndpoints, useAppsecServices } from './queries';

const WRITES = ['POST', 'PUT', 'PATCH', 'DELETE'];

function useQueryParam() {
  const [params, setParams] = useSearchParams();
  const q = params.get('q') ?? '';
  const setQ = (v: string) => setParams((p) => (v ? p.set('q', v) : p.delete('q'), p), { replace: true });
  return [q, setQ] as const;
}

/** The free text of a query, lowercased (facet terms are handled by the facets). */
const textOf = (terms: Term[]) =>
  terms
    .filter((t) => t.kind === 'text')
    .map((t) => (t.kind === 'text' ? t.text : ''))
    .join(' ')
    .trim()
    .toLowerCase();

function Card({ title, help, children }: { title: string; help?: string; children: ReactNode }) {
  return (
    <div className="inv-card">
      <h5>
        {title}
        {help && (
          <span className="inv-help" title={help}>
            <HelpCircle size={13} />
          </span>
        )}
      </h5>
      <p>{children}</p>
    </div>
  );
}

/** Inventory → API Endpoints: every endpoint of the traces, with exposure and findings. */
export function EndpointsPage() {
  const q = useApiEndpoints();
  const [query, setQuery] = useQueryParam();
  const [draft, setDraft] = useState(query);
  const [controls, setControls] = useState(true);
  const terms = parseQuery(query);
  const all = q.data?.endpoints ?? [];
  const text = textOf(terms);
  const searched = all.filter((e) => !text || `${e.method} ${e.route} ${e.service} ${e.hosts.join(' ')}`.toLowerCase().includes(text));
  const { facets, shown: rows } = localFacets<ApiEndpointView>(
    searched,
    [
      { key: 'source', label: 'Source', group: 'CORE', get: (e) => (e.protected ? ['App & API Protection', 'Distributed Tracing'] : ['Distributed Tracing']) },
      { key: 'service', label: 'Service', group: 'OVERVIEW', get: (e) => [e.service] },
      { key: 'env', label: 'Env', group: 'OVERVIEW', get: (e) => (e.env ? [e.env] : []) },
      { key: '@http.method', label: 'Method', group: 'OVERVIEW', get: (e) => [e.method] },
      { key: 'exposure', label: 'Exposure', group: 'OVERVIEW', get: (e) => [...(e.attacks ? ['Exposed to attacks'] : []), ...(e.authenticated ? ['Authenticated'] : ['Unauthenticated'])] },
    ],
    terms,
  );
  const findings = (s: string) => all.reduce((a, e) => a + (e.findings[s] ?? 0), 0);
  return (
    <div className="sec-explorer inv-page">
      <AapTabs tabs={INVENTORY_TABS} />
      <div className="inv-cards six">
        <Card title="Not protected" help="Endpoints of services that don't run App & API Protection">
          <b>{compact(all.filter((e) => !e.protected).length)}</b> endpoints
        </Card>
        <Card title="Unauthenticated writes" help="POST, PUT, PATCH or DELETE endpoints that never saw an authenticated user">
          <b>{compact(all.filter((e) => !e.authenticated && WRITES.includes(e.method)).length)}</b> endpoints
        </Card>
        <Card title="With business logic">
          <b>{compact(all.filter((e) => e.loginEvents > 0).length)}</b> tracked
        </Card>
        <Card title="Blocking attacks">
          <b>{compact(all.filter((e) => e.blocked > 0).length)}</b> endpoints
        </Card>
        <Card title="Exposed to attacks">
          <b>{compact(all.filter((e) => e.attacks > 0).length)}</b> endpoints
        </Card>
        <Card title="With security findings">
          <b>{findings('critical')}</b> Criticals <b>{findings('high')}</b> Highs <b>{compact(findings('medium'))}</b> Mediums
        </Card>
      </div>
      <form
        className="inv-search wide"
        onSubmit={(e) => {
          e.preventDefault();
          setQuery(draft.trim());
        }}
      >
        <Search size={15} />
        <input placeholder="Search APIs..." value={draft} onChange={(e) => setDraft(e.target.value)} onBlur={() => setQuery(draft.trim())} />
        <span className="inv-search-code" title="Search syntax: free text or facet:value">
          <Code2 size={14} />
        </span>
      </form>
      <div className={`syn-explorer-body${controls ? '' : ' no-controls'}`}>
        {controls && <FacetPanel facets={facets} terms={terms} onChange={(t: Term[]) => setQuery(formatQuery(t))} defaultOpen={['source', 'service']} verified={[]} noun="facets" valueSearch />}
        <div className="syn-explorer-main">
          <ExplorerToolbar controls={controls} onToggle={() => setControls((c) => !c)}>
            <span>
              <b>{compact(rows.length)}</b> Endpoints
            </span>
          </ExplorerToolbar>
          {q.isLoading ? (
            <Loading />
          ) : q.isError ? (
            <ErrorState error={q.error} />
          ) : !rows.length ? (
            <Empty>
              <b>No endpoints.</b>
              <p className="muted">Endpoints come from the server spans of your traced services.</p>
            </Empty>
          ) : (
            <table className="mon-table syn-table inv-table">
              <thead>
                <tr>
                  <th className="sec-check" />
                  <th>NAME</th>
                  <th>SOURCE</th>
                  <th>SERVICE</th>
                  <th>HOSTNAMES</th>
                  <th className="num">REQUESTS (7D)</th>
                  <th className="num">ATTACKS</th>
                  <th>FINDINGS</th>
                  <th>LAST SEEN</th>
                </tr>
              </thead>
              <tbody>
                {rows.map((e) => (
                  <tr key={`${e.service}${e.method}${e.route}`}>
                    <td className="sec-check">
                      <input type="checkbox" aria-label={`${e.method} ${e.route}`} />
                    </td>
                    <td>
                      <MethodPath method={e.method} route={e.route} />
                    </td>
                    <td className="inv-sources">
                      {e.protected && (
                        <span title="App & API Protection">
                          <Shield size={14} />
                        </span>
                      )}
                      <span title="Distributed Tracing">
                        <ListTree size={14} />
                      </span>
                    </td>
                    <td>
                      <ServiceChip name={e.service} />
                    </td>
                    <td>
                      {e.hosts.slice(0, 2).map((h) => (
                        <span key={h} className="inv-chip">
                          {h}
                        </span>
                      ))}
                      {e.hosts.length > 2 && <span className="inv-more">+{e.hosts.length - 2}</span>}
                    </td>
                    <td className="num">{compact(e.hits)}</td>
                    <td className="num">
                      {e.attacks ? <Link to={`/security/appsec/investigate?q=${encodeURIComponent(`service:${e.service} @http.route:"${e.route}"`)}`}>{compact(e.attacks)}</Link> : 0}
                    </td>
                    <td className="nowrap">
                      {['critical', 'high', 'medium', 'low'].map((s) =>
                        e.findings[s] ? (
                          <span key={s} className={`sec-sev sev-${s} aap-sev-n`}>
                            {e.findings[s]}
                          </span>
                        ) : null,
                      )}
                    </td>
                    <td className="muted nowrap">{timeAgo(e.lastSeen)}</td>
                  </tr>
                ))}
              </tbody>
            </table>
          )}
        </div>
      </div>
    </div>
  );
}

const COVERAGE: [keyof AppsecService, string, string][] = [
  ['protected', 'AAP', 'Threat protection'],
  ['sca', 'SCA', 'Software Composition Analysis'],
  ['iast', 'IAST', 'Runtime Code Analysis'],
];

/** Inventory → Services: App & API Protection's activation and mode per service. */
export function AppsecServicesPage() {
  const q = useAppsecServices();
  const [query, setQuery] = useQueryParam();
  const [draft, setDraft] = useState(query);
  const [controls, setControls] = useState(true);
  const { stars, toggle } = useStars('oc.appsec.services.stars');
  const terms = parseQuery(query);
  const all = q.data ?? [];
  const text = textOf(terms);
  const { facets, shown } = localFacets<AppsecService>(
    all.filter((s) => !text || s.service.toLowerCase().includes(text)),
    [
      { key: 'env', label: 'Environment', group: 'CORE', get: (s) => [s.env] },
      { key: 'mode', label: 'Mode', group: 'CORE', get: (s) => [s.mode] },
      { key: 'language', label: 'Language', group: 'CORE', get: (s) => [LANGUAGE_LABEL[s.language] ?? s.language] },
      { key: 'policy', label: 'Policy', group: 'CORE', get: (s) => [s.policy] },
    ],
    terms,
  );
  const rows = [...shown].sort((a, b) => Number(stars.has(b.service)) - Number(stars.has(a.service)) || b.securityTraces - a.securityTraces);
  const cards: [string, number, string][] = [
    ['App & API Protection in Monitoring Mode', all.filter((s) => s.mode === 'monitoring').length, 'of'],
    ['App & API Protection in Blocking Mode', all.filter((s) => s.mode === 'blocking').length, 'across'],
    ['Software Composition Analysis', all.filter((s) => s.sca).length, 'across'],
    ['Runtime Code Analysis', all.filter((s) => s.iast).length, 'across'],
  ];
  const csv = () =>
    downloadCsv(
      'appsec-services.csv',
      ['service', 'env', 'language', 'mode', 'policy', 'vulnerabilities', 'security_traces'],
      rows.map((s) => [s.service, s.env, s.language, s.mode, s.policy, s.vulnerabilities, s.securityTraces]),
    );
  return (
    <div className="sec-explorer inv-page">
      <AapTabs tabs={INVENTORY_TABS} />
      <div className="inv-cards four">
        {cards.map(([t, n, word]) => (
          <div key={t} className="inv-card">
            <h5>{t}</h5>
            <p>
              <b>{n}</b> {word} <b>{all.length}</b> compatible services
            </p>
            <div className="inv-progress">
              <i style={{ width: `${all.length ? (n / all.length) * 100 : 0}%` }} />
            </div>
          </div>
        ))}
      </div>
      <form
        className="inv-search wide"
        onSubmit={(e) => {
          e.preventDefault();
          setQuery(draft.trim());
        }}
      >
        <span className="inv-search-label">Search for</span>
        <Search size={15} />
        <input placeholder="Search by service name or facets" value={draft} onChange={(e) => setDraft(e.target.value)} onBlur={() => setQuery(draft.trim())} />
      </form>
      <div className={`syn-explorer-body${controls ? '' : ' no-controls'}`}>
        {controls && <FacetPanel facets={facets} terms={terms} onChange={(t: Term[]) => setQuery(formatQuery(t))} defaultOpen={['env', 'mode', 'language']} verified={[]} noun="facets" />}
        <div className="syn-explorer-main">
          <ExplorerToolbar controls={controls} onToggle={() => setControls((c) => !c)}>
            <span>
              <b>{rows.length}</b> services
            </span>
            <span className="inv-sep" />
            <button className="btn btn-sm" onClick={csv} disabled={!rows.length}>
              <Download size={13} /> Export as CSV
            </button>
          </ExplorerToolbar>
          {q.isLoading ? (
            <Loading />
          ) : q.isError ? (
            <ErrorState error={q.error} />
          ) : (
            <table className="mon-table syn-table inv-table">
              <thead>
                <tr>
                  <th className="sec-check" />
                  <th className="inv-star-col">
                    <Star size={13} />
                  </th>
                  <th>SERVICE</th>
                  <th>ENVIRONMENT</th>
                  <th className="num">VULNERABILITY RISK</th>
                  <th>TREND</th>
                  <th>COVERAGE</th>
                </tr>
              </thead>
              <tbody>
                {rows.map((s) => (
                  <tr key={s.service}>
                    <td className="sec-check">
                      <input type="checkbox" aria-label={s.service} />
                    </td>
                    <td className="inv-star-col">
                      <button className={`inv-star${stars.has(s.service) ? ' on' : ''}`} onClick={() => toggle(s.service)} aria-label={stars.has(s.service) ? 'Unstar' : 'Star'}>
                        <Star size={14} />
                      </button>
                    </td>
                    <td>
                      <span className="inv-svc-name">
                        <ServiceChip name={s.service} />
                        {s.language && <span className="inv-lang">{LANGUAGE_LABEL[s.language] ?? s.language}</span>}
                      </span>
                    </td>
                    <td>
                      <span className="inv-env">{s.env}</span>
                    </td>
                    <td className="num">{s.vulnerabilities || ''}</td>
                    <td>
                      <span className="inv-trend">
                        <b>{compact(s.securityTraces)}</b>
                        <ActivitySpark values={s.trend} />
                      </span>
                    </td>
                    <td className="nowrap">
                      {COVERAGE.map(([k, label, title]) => (
                        <span
                          key={label}
                          className={`inv-cov${s[k] ? ' on' : ''}${k === 'protected' && s.mode === 'blocking' ? ' blocking' : ''}`}
                          title={k === 'protected' && s[k] ? `${title}: ${s.mode} mode, ${s.policy}` : `${title}: ${s[k] ? 'enabled' : 'not enabled'}`}
                        >
                          {label}
                        </span>
                      ))}
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          )}
        </div>
      </div>
    </div>
  );
}
