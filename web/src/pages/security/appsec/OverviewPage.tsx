import { useMemo, useState, type ReactNode } from 'react';
import { Link, useSearchParams } from 'react-router-dom';
import {
  AlertTriangle,
  ArrowRight,
  Braces,
  ChevronDown,
  ChevronRight,
  ExternalLink,
  Filter,
  Globe,
  Hand,
  LibraryBig,
  PanelLeftClose,
  PanelLeftOpen,
  Search,
  TrendingDown,
  TrendingUp,
} from 'lucide-react';
import type { ApiFinding, AppsecService, TrendCount } from '../../../api/appsec/client';
import { OWASP } from '../../../api/appsec/client';
import type { SecuritySignal } from '../../../api/security/client';
import { TimePicker } from '../../../components/shell/PageHeader';
import { useTime } from '../../../components/shell/TimeContext';
import { ErrorState, Loading } from '../../../components/ui/AsyncState';
import { compact } from '../../../lib/format';
import { SeverityBadge } from '../common';
import { useSignals } from '../queries';
import { MethodBadge } from './AppsecLayout';
import { useApiEndpoints, useApiFindings, useAppsecOverview, useAppsecServices } from './queries';
import './overview.css';

const MIN = 60_000;
const DOCS = 'https://github.com/observecraft/observecraft/blob/main/docs/sending-data.md';
type View = 'posture' | 'threat';
const NAV: [View, string, typeof Braces, [string, string][]][] = [
  [
    'posture',
    'API Posture',
    Braces,
    [
      ['coverage', 'Coverage'],
      ['findings', 'Open Findings'],
      ['exposure', 'Threats Exposure'],
      ['policies', 'Policies Coverage'],
    ],
  ],
  [
    'threat',
    'Threat Protection',
    Hand,
    [
      ['instrumentation', 'App Instrumentation'],
      ['services-exposure', 'Threats Exposure'],
      ['signals', 'Open Signals'],
      ['attack-coverage', 'Attack Coverage'],
      ['trends', 'Threat Trends'],
    ],
  ],
];
const SEV_GROUPS: [string[], string][] = [
  [['critical'], 'critical'],
  [['high'], 'high'],
  [['medium', 'low'], 'medium'],
];
const SEV_COLOR: Record<string, string> = { critical: 'var(--sev-critical)', high: 'var(--sev-high)', medium: 'var(--sev-medium)', low: 'var(--sev-low)', info: 'var(--sev-info)' };
const RANK = ['critical', 'high', 'medium', 'low', 'info'];
const WRITES = ['POST', 'PUT', 'PATCH', 'DELETE'];
const regions = (() => {
  try {
    return new Intl.DisplayNames(['en'], { type: 'region' });
  } catch {
    return null;
  }
})();
/** A country's name from its ISO code. */
const country = (iso: string) => {
  try {
    return (iso.length === 2 && regions?.of(iso.toUpperCase())) || iso;
  } catch {
    return iso;
  }
};

type Keep = (s: { env: string; service: string }) => boolean;

/** App & API Protection → Overview: API Posture and Threat Protection, each its own view. */
export function AppsecOverviewPage() {
  const [params, setParams] = useSearchParams();
  const view: View = params.get('view') === 'threat' ? 'threat' : 'posture';
  const [active, setActive] = useState(view === 'threat' ? 'instrumentation' : 'coverage');
  const [collapsed, setCollapsed] = useState(false);
  const [env, setEnv] = useState('');
  const [service, setService] = useState('');
  const services = useAppsecServices();
  const endpoints = useApiEndpoints();
  const allServices = services.data ?? [];
  const envs = [...new Set([...allServices.map((s) => s.env), ...(endpoints.data?.endpoints ?? []).map((e) => e.env)].filter(Boolean))].sort();
  const names = [...new Set(allServices.map((s) => s.service))].sort();
  const go = (v: View, id: string) => {
    setActive(id);
    if (v !== view) {
      setParams(v === 'threat' ? { view: 'threat' } : {}, { replace: true });
      setTimeout(() => document.getElementById(`aap-${id}`)?.scrollIntoView({ block: 'start' }), 50);
    } else document.getElementById(`aap-${id}`)?.scrollIntoView({ behavior: 'smooth', block: 'start' });
  };
  const keep: Keep = (s) => (!env || s.env === env) && (!service || s.service === service);
  return (
    <div className={`ov-layout${collapsed ? ' collapsed' : ''}`}>
      <aside className="ov-nav">
        <div className="ov-nav-groups">
          {NAV.map(([v, label, Icon, items]) => (
            <div key={v} className="ov-nav-group">
              <button className={`ov-nav-head${view === v ? ' on' : ''}`} onClick={() => go(v, items[0][0])} title={label}>
                <Icon size={15} /> {!collapsed && label}
              </button>
              {!collapsed &&
                view === v &&
                items.map(([id, l]) => (
                  <button key={id} className={`ov-nav-item${active === id ? ' active' : ''}`} onClick={() => go(v, id)}>
                    {l}
                  </button>
                ))}
            </div>
          ))}
        </div>
        <div className="ov-nav-foot">
          <a href={DOCS} target="_blank" rel="noreferrer" title="Documentation">
            <LibraryBig size={15} /> {!collapsed && 'Documentation'} {!collapsed && <ExternalLink size={13} className="ov-push" />}
          </a>
          <button onClick={() => setCollapsed((c) => !c)} title={collapsed ? 'Expand' : 'Collapse'}>
            {collapsed ? <PanelLeftOpen size={15} /> : <PanelLeftClose size={15} />} {!collapsed && 'Collapse'}
          </button>
        </div>
      </aside>
      <div className="ov-main">
        <div className="ov-filters">
          <span className="ov-filter-by">
            <Filter size={14} /> Filter by
          </span>
          <label className="ov-select">
            <span>Env</span>
            <select value={env} onChange={(e) => setEnv(e.target.value)} aria-label="Env">
              <option value="">All</option>
              {envs.map((e) => (
                <option key={e}>{e}</option>
              ))}
            </select>
          </label>
          <label className="ov-select">
            <span>Service</span>
            <select value={service} onChange={(e) => setService(e.target.value)} aria-label="Service">
              <option value="">All</option>
              {names.map((s) => (
                <option key={s}>{s}</option>
              ))}
            </select>
          </label>
          <span className="syn-grow" />
          <TimePicker compact />
        </div>
        {view === 'posture' ? <ApiPosture keep={keep} endpoints={endpoints} /> : <ThreatProtection keep={keep} services={allServices} env={env} service={service} />}
      </div>
    </div>
  );
}

function ApiPosture({ keep, endpoints }: { keep: Keep; endpoints: ReturnType<typeof useApiEndpoints> }) {
  const findingsQ = useApiFindings();
  if (endpoints.isLoading || findingsQ.isLoading) return <Loading />;
  if (endpoints.isError) return <ErrorState error={endpoints.error} onRetry={() => endpoints.refetch()} />;
  if (findingsQ.isError) return <ErrorState error={findingsQ.error} onRetry={() => findingsQ.refetch()} />;
  const eps = (endpoints.data?.endpoints ?? []).filter(keep);
  const page = findingsQ.data!;
  const findings = page.findings.filter(keep);
  const open = findings.filter((f) => f.status === 'open');
  const muted = findings.filter((f) => f.status === 'muted');
  const count = (list: ApiFinding[], sevs: string[]) => list.filter((f) => sevs.includes(f.severity)).length;
  const unprotected = eps.filter((e) => !e.protected).length;
  const authenticated = eps.filter((e) => e.authenticated > 0).length;
  const unauthWrites = eps.filter((e) => e.authenticated === 0 && WRITES.includes(e.method)).length;
  const undetected = eps.length - authenticated - unauthWrites;
  const attacked = eps.filter((e) => e.attacks > 0);
  const byService = new Map<string, number>();
  for (const e of attacked) byService.set(e.service, (byService.get(e.service) ?? 0) + e.attacks);
  const topServices = [...byService.entries()].sort((a, b) => b[1] - a[1]).slice(0, 5);
  const rules = new Map<string, { name: string; severity: string; endpoints: number }>();
  for (const f of open) {
    const r = rules.get(f.ruleId) ?? { name: f.ruleName, severity: f.severity, endpoints: 0 };
    r.endpoints += 1;
    rules.set(f.ruleId, r);
  }
  const topRules = [...rules.values()].sort((a, b) => RANK.indexOf(a.severity) - RANK.indexOf(b.severity) || b.endpoints - a.endpoints).slice(0, 5);
  const exposure = eps
    .map((e) => ({ e, crit: e.findings.critical ?? 0, high: e.findings.high ?? 0 }))
    .filter((x) => x.crit + x.high > 0)
    .sort((a, b) => b.crit - a.crit || b.high - a.high)
    .slice(0, 5);
  const maxExposure = Math.max(1, ...exposure.map((x) => x.crit + x.high));
  const covered = new Set(page.rules.map((r) => r.owasp));
  const owaspPct = Math.round((OWASP.filter(([id]) => covered.has(id)).length / OWASP.length) * 100);
  return (
    <>
      <h2 className="ov-title">API Posture</h2>
      <Section id="coverage" title="Coverage">
        <div className="ov-cards three">
          <div className="ov-card">
            <h5>
              INVENTORY{' '}
              <Link to="/security/appsec/inventory">
                View Endpoints <ArrowRight size={12} />
              </Link>
            </h5>
            <div className="ov-donut-row">
              <Ring
                total={eps.length}
                parts={[
                  [eps.length - unprotected, '#5b2fb8'],
                  [unprotected, '#b9a3e8'],
                ]}
              />
              <ul className="ov-legend">
                <Legend color="#5b2fb8" label="Protected" value={eps.length - unprotected} />
                <Legend color="#b9a3e8" label="Not protected" value={unprotected} warn />
              </ul>
            </div>
          </div>
          <div className="ov-card">
            <h5>AUTHENTICATION POSTURE</h5>
            <div className="ov-donut-row">
              <Ring
                total={eps.length}
                parts={[
                  [authenticated, '#1c5fc4'],
                  [unauthWrites, '#6fa5e8'],
                  [undetected, '#dde3ee'],
                ]}
              />
              <ul className="ov-legend">
                <Legend color="#1c5fc4" label="Authenticated" value={authenticated} />
                <Legend color="#6fa5e8" label="Unauthenticated writes" value={unauthWrites} warn />
                <Legend color="#dde3ee" label="Undetected" value={undetected} warn />
              </ul>
            </div>
          </div>
          <div className="ov-card">
            <h5>TOP SERVICES BY ATTACKS</h5>
            {topServices.length ? <Bars rows={topServices.map(([name, count]) => ({ name, count, change: null }))} shades /> : <p className="muted">No attacks on these endpoints.</p>}
          </div>
        </div>
        <div className="ov-exposure">
          <h5>
            <AlertTriangle size={13} /> ACTIVE EXPOSURE
          </h5>
          <div>
            <span>
              Exposed to attacks <b>{attacked.length}</b>
            </span>
            <span>
              Unauthenticated writes <b>{unauthWrites}</b>
            </span>
            <span>
              Not protected <b>{unprotected}</b>
            </span>
          </div>
        </div>
      </Section>
      <Section id="findings" title="Open Findings" link={['View Findings', '/security/appsec/findings']}>
        <div className="ov-split">
          <div className="ov-sev-cards">
            {SEV_GROUPS.map(([sevs, cls]) => (
              <Link key={cls} className="ov-sev-card" to={`/security/appsec/findings?severity=${sevs.join(',')}`}>
                <div>
                  <b className={`sev-text-${cls}`}>{compact(count(open, sevs))}</b>
                  <span>
                    {sevs.map((s) => (
                      <SeverityBadge key={s} severity={s} />
                    ))}
                  </span>
                </div>
                <small>{count(muted, sevs)} muted</small>
              </Link>
            ))}
          </div>
          <div className="ov-card">
            <h5>
              TOP LATEST FINDINGS{' '}
              <Link to="/security/appsec/findings?severity=critical,high">
                View all Critical &amp; High <ArrowRight size={12} />
              </Link>
            </h5>
            {topRules.length ? (
              <table className="ov-table boxed">
                <thead>
                  <tr>
                    <th>RULE</th>
                    <th>ENDPOINTS</th>
                    <th>TICKETS</th>
                  </tr>
                </thead>
                <tbody>
                  {topRules.map((r) => (
                    <tr key={r.name}>
                      <td>
                        <i className="ov-sev-bar" style={{ background: SEV_COLOR[r.severity] }} />
                        {r.name}
                      </td>
                      <td>{compact(r.endpoints)}</td>
                      <td />
                    </tr>
                  ))}
                </tbody>
              </table>
            ) : (
              <p className="muted">No open findings.</p>
            )}
          </div>
        </div>
      </Section>
      <Section id="exposure" title="Threats Exposure" sub="Top endpoints by critical findings" link={['View Endpoints', '/security/appsec/inventory']}>
        {exposure.length ? (
          <table className="ov-table boxed">
            <thead>
              <tr>
                <th>ENDPOINT</th>
                <th>SERVICE</th>
                <th>TEAM</th>
                <th>FINDINGS COUNT</th>
              </tr>
            </thead>
            <tbody>
              {exposure.map(({ e, crit, high }) => (
                <tr key={`${e.service}${e.method}${e.route}`}>
                  <td>
                    <MethodBadge method={e.method} /> <b>{e.route}</b>
                  </td>
                  <td>
                    <ServiceChip name={e.service} />
                  </td>
                  <td className="muted">—</td>
                  <td>
                    <span className="ov-count">
                      {crit + high}
                      <span className="ov-seg" style={{ width: `${((crit + high) / maxExposure) * 72}px` }}>
                        <i style={{ flex: crit, background: 'var(--sev-critical)' }} />
                        <i style={{ flex: high, background: 'var(--sev-high)' }} />
                      </span>
                    </span>
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        ) : (
          <Empty text="No endpoints with critical or high findings" boxed />
        )}
      </Section>
      <Section id="policies" title="Policies Coverage" link={['View Policies', '/security/appsec/policies/rules']}>
        <div className="ov-card">
          <h5>API FINDING RULES</h5>
          <div className="ov-nums">
            <span>
              <b>{page.rules.length}</b> Enabled
            </span>
            <span>
              <b>0</b> Disabled
            </span>
          </div>
        </div>
        <div className="ov-card ov-owasp">
          <h5>POLICIES ENABLED BY OWASP API TOP 10</h5>
          <div className="ov-owasp-head">
            <b>{owaspPct} %</b>
            <span className="muted">{page.rules.length} Rules</span>
          </div>
          <div className="ov-owasp-line">
            <i style={{ width: `${owaspPct}%` }} />
          </div>
          <div className="ov-owasp-blocks">
            {OWASP.map(([id, name]) => (
              <span key={id} className={covered.has(id) ? 'on' : ''} title={`${id}: ${name}`}>
                <i />
                {id}
              </span>
            ))}
          </div>
        </div>
      </Section>
    </>
  );
}

function ThreatProtection({ keep, services, env, service }: { keep: Keep; services: AppsecService[]; env: string; service: string }) {
  const { range } = useTime();
  const r = useMemo(() => ({ from: Math.floor(range.from / MIN) * MIN, to: Math.ceil(range.to / MIN) * MIN }), [range.from, range.to]);
  const prev = useMemo(() => ({ from: r.from - (r.to - r.from), to: r.from }), [r]);
  const q = useAppsecOverview(r);
  const now = useSignals({ range: r, query: '@workflow.rule.type:application_security', limit: 500 });
  const before = useSignals({ range: prev, query: '@workflow.rule.type:application_security', limit: 500 });
  if (q.isLoading) return <Loading />;
  if (q.isError) return <ErrorState error={q.error} onRetry={() => q.refetch()} />;
  const o = q.data!;
  const inScope = (s: SecuritySignal) => (!env || s.env === env) && (!service || s.services.includes(service));
  const sig = (now.data?.signals ?? []).filter(inScope);
  const open = sig.filter((s) => s.state === 'open');
  const triaged = sig.filter((s) => s.state === 'under_review');
  const bySev = (list: SecuritySignal[], sevs: string[]) => list.filter((x) => sevs.includes(x.severity)).length;
  const prevByRule = new Map<string, number>();
  for (const s of (before.data?.signals ?? []).filter(inScope)) if (s.state === 'open') prevByRule.set(s.ruleName, (prevByRule.get(s.ruleName) ?? 0) + 1);
  const byRule = new Map<string, { severity: string; n: number }>();
  for (const s of open) {
    const g = byRule.get(s.ruleName) ?? { severity: s.severity, n: 0 };
    g.n += 1;
    byRule.set(s.ruleName, g);
  }
  const byService = new Map<string, { env: string; n: number }>();
  for (const s of open) for (const name of s.services) byService.set(name, { env: s.env, n: (byService.get(name)?.n ?? 0) + 1 });
  const svc = services.filter(keep);
  const withTraffic = svc.length;
  const protectedN = svc.filter((s) => s.protected).length;
  const blocking = svc.filter((s) => s.mode === 'blocking').length;
  const monitoring = svc.filter((s) => s.mode === 'monitoring').length;
  const pct = (a: number, b: number) => (b ? Math.round((a / b) * 100) : 0);
  return (
    <>
      <Section id="instrumentation" title="App Instrumentation" sub="Track App & API Protection activation across your services">
        <div className="ov-cards two">
          <div className="ov-card">
            <h5>THREAT DETECTION COVERAGE</h5>
            <b className="ov-pct">{pct(protectedN, withTraffic)}%</b>
            <div className="ov-progress">
              <i style={{ width: `${pct(protectedN, withTraffic)}%` }} />
            </div>
            <div className="ov-card-foot">
              <span className="muted">
                {protectedN} / {withTraffic} services with traffic
              </span>
              <Link className="btn ov-btn" to="/security/appsec/inventory/services">
                Protect Additional Services <ArrowRight size={13} />
              </Link>
            </div>
          </div>
          <div className="ov-card">
            <h5>SERVICES IN BLOCKING MODE</h5>
            <b className="ov-pct">{pct(blocking, protectedN)}%</b>
            <div className="ov-progress">
              <i style={{ width: `${pct(blocking, protectedN)}%` }} />
            </div>
            <div className="ov-card-foot">
              <span className="muted">
                {blocking} / {protectedN} protected services
              </span>
              <Link className="btn btn-primary ov-btn" to="/security/appsec/policies">
                Review Policies <ArrowRight size={13} />
              </Link>
            </div>
          </div>
        </div>
      </Section>
      <Section id="services-exposure" title="Threats Exposure" sub="Top services by signals">
        <table className="ov-table boxed">
          <thead>
            <tr>
              <th>SERVICE</th>
              <th>ENV</th>
              <th>TEAM</th>
              <th>SIGNALS COUNT</th>
            </tr>
          </thead>
          <tbody>
            {[...byService.entries()]
              .sort((a, b) => b[1].n - a[1].n)
              .slice(0, 5)
              .map(([name, v]) => (
                <tr key={name}>
                  <td>
                    <ServiceChip name={name} />
                  </td>
                  <td>{v.env || '—'}</td>
                  <td className="muted">—</td>
                  <td>{v.n}</td>
                </tr>
              ))}
            {!byService.size && (
              <tr>
                <td colSpan={4}>
                  <Empty text="No services with attacks" />
                </td>
              </tr>
            )}
          </tbody>
        </table>
      </Section>
      <Section id="signals" title="Open Signals" link={['View Signals', '/security/appsec/signals']}>
        <div className="ov-split">
          <div className="ov-sev-cards">
            {SEV_GROUPS.map(([sevs, cls]) => (
              <Link key={cls} className="ov-sev-card" to="/security/appsec/signals">
                <div>
                  <b className={`sev-text-${cls}`}>{bySev(open, sevs)}</b>
                  <span>
                    {sevs.map((s) => (
                      <SeverityBadge key={s} severity={s} />
                    ))}
                  </span>
                </div>
                <small>{bySev(triaged, sevs)} triaged</small>
              </Link>
            ))}
          </div>
          <div className="ov-card">
            <h5>
              RECENT OPEN SIGNALS GROUPED BY RULE{' '}
              <Link to="/security/appsec/signals">
                View All Grouped <ArrowRight size={12} />
              </Link>
            </h5>
            {byRule.size ? (
              <table className="ov-table boxed">
                <thead>
                  <tr>
                    <th>RULE</th>
                    <th>SIGNALS</th>
                  </tr>
                </thead>
                <tbody>
                  {[...byRule.entries()]
                    .sort((a, b) => b[1].n - a[1].n)
                    .slice(0, 5)
                    .map(([rule, g]) => {
                      const p = prevByRule.get(rule);
                      return (
                        <tr key={rule}>
                          <td>
                            <i className="ov-sev-bar" style={{ background: SEV_COLOR[g.severity] }} />
                            {rule}
                          </td>
                          <td>
                            {g.n} {p ? <Change value={Math.round(((g.n - p) / p) * 100)} /> : null}
                          </td>
                        </tr>
                      );
                    })}
                </tbody>
              </table>
            ) : (
              <p className="muted">No open signals.</p>
            )}
          </div>
        </div>
      </Section>
      <Section id="attack-coverage" title="Attack Coverage" sub="Coverage of exposed services by active In-App WAF rules" link={['View Policies', '/security/appsec/policies']}>
        <div className="ov-card">
          <h5>ATTACK TOOLS</h5>
          <div className="ov-cov-head">
            <b>{pct(protectedN, withTraffic)}% services covered</b>
            <span className="muted">
              {protectedN} / {withTraffic} exposed
            </span>
          </div>
          <div className="ov-stack">
            <i className="blocking" style={{ flex: blocking }} />
            <i className="monitoring" style={{ flex: monitoring }} />
            <i className="off" style={{ flex: withTraffic - protectedN }} />
          </div>
          <div className="ov-cov-legend">
            <span>
              <i className="blocking" /> Blocking ({blocking})
            </span>
            <span>
              <i className="monitoring" /> Monitoring ({monitoring})
            </span>
            {withTraffic > protectedN && (
              <span>
                <i className="off" /> App &amp; API Protection isn&rsquo;t enabled ({withTraffic - protectedN})
              </span>
            )}
          </div>
          {withTraffic > protectedN && (
            <Link className="ov-center-link" to="/security/appsec/inventory/services">
              Enable in Services <ArrowRight size={12} />
            </Link>
          )}
        </div>
      </Section>
      <Section id="trends" title="Threat Trends" link={['View All in Traces Explorer', '/security/appsec/investigate']}>
        <div className="ov-cards two">
          <div className="ov-card">
            <h5>
              TOP ATTACK TYPES{' '}
              <Link to="/security/appsec/investigate">
                Investigate <ArrowRight size={12} />
              </Link>
            </h5>
            <Bars rows={o.topAttackTypes} shades />
          </div>
          <div className="ov-card">
            <h5>
              TOP COUNTRIES{' '}
              <Link to="/security/appsec/investigate">
                Investigate <ArrowRight size={12} />
              </Link>
            </h5>
            <Bars rows={o.topCountries.map((c) => ({ ...c, name: country(c.name) }))} shades />
          </div>
        </div>
      </Section>
    </>
  );
}

function Section({ id, title, sub, link, children }: { id: string; title: string; sub?: string; link?: [string, string]; children: ReactNode }) {
  const [open, setOpen] = useState(true);
  return (
    <section id={`aap-${id}`} className="ov-section">
      <header>
        <button className="link-btn" onClick={() => setOpen((o) => !o)} aria-label={open ? 'Collapse' : 'Expand'}>
          {open ? <ChevronDown size={18} /> : <ChevronRight size={18} />}
        </button>
        <h3>{title}</h3>
        {sub && <span className="muted">{sub}</span>}
        {link && (
          <Link className="ov-section-link" to={link[1]}>
            {link[0]} <ArrowRight size={14} />
          </Link>
        )}
      </header>
      {open && <div className="ov-section-body">{children}</div>}
    </section>
  );
}

/** A thin ring with the total in the middle. */
function Ring({ total, parts }: { total: number; parts: [number, string][] }) {
  const sum = parts.reduce((a, [n]) => a + n, 0);
  const R = 42;
  const C = 2 * Math.PI * R;
  let acc = 0;
  return (
    <svg className="ov-ring" viewBox="0 0 100 100" role="img" aria-label={`${total}`}>
      <circle cx="50" cy="50" r={R} fill="none" stroke="var(--bg-group)" strokeWidth="7" />
      {sum > 0 &&
        parts.map(([n, color], i) => {
          const len = (n / sum) * C;
          const el = <circle key={i} cx="50" cy="50" r={R} fill="none" stroke={color} strokeWidth="7" strokeDasharray={`${len} ${C - len}`} strokeDashoffset={-acc} transform="rotate(-90 50 50)" />;
          acc += len;
          return el;
        })}
      <text x="50" y="50" textAnchor="middle" dominantBaseline="central">
        {compact(total)}
      </text>
    </svg>
  );
}

function Legend({ color, label, value, warn }: { color: string; label: string; value: number; warn?: boolean }) {
  return (
    <li>
      <i style={{ background: color }} /> {label} – <b>{compact(value)}</b> {warn && value > 0 && <AlertTriangle size={13} className="ov-warn" />}
    </li>
  );
}

function ServiceChip({ name }: { name: string }) {
  return (
    <span className="ov-service">
      <i>
        <Globe size={12} />
      </i>
      {name}
    </span>
  );
}

function Empty({ text, boxed }: { text: string; boxed?: boolean }) {
  return (
    <div className={`ov-empty${boxed ? ' boxed' : ''}`}>
      <Search size={36} />
      <span>{text}</span>
    </div>
  );
}

function Change({ value }: { value: number }) {
  return (
    <em className={`ov-change ${value > 0 ? 'up' : 'down'}`}>
      {value > 0 ? '+' : ''}
      {value}% {value > 0 ? <TrendingUp size={12} /> : <TrendingDown size={12} />}
    </em>
  );
}

/** Horizontal bars: label, bar, value and change against the previous period. */
export function Bars({ rows, shades }: { rows: TrendCount[]; shades?: boolean }) {
  const max = Math.max(1, ...rows.map((r) => r.count));
  if (!rows.length) return <p className="muted">No data</p>;
  return (
    <ul className="ov-bars">
      {rows.map((r, i) => (
        <li key={r.name}>
          <span title={r.name}>{r.name}</span>
          <span className="ov-bar-cell">
            <i style={{ width: `${(r.count / max) * 75}%`, opacity: shades ? 1 - i * 0.16 : 1 }} />
            <b>{compact(r.count)}</b>
            {r.change != null && <Change value={r.change} />}
          </span>
        </li>
      ))}
    </ul>
  );
}
