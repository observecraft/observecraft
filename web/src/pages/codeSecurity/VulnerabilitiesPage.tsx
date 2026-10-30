import { useState } from 'react';
import { keepPreviousData, useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { Link, useNavigate, useParams, useSearchParams } from 'react-router-dom';
import { ArrowRight, BellOff, Bell, Code2, ExternalLink, GitBranch, Globe, KeyRound, Layers, Rocket, ScanEye, Search, ShieldCheck, Wrench, X } from 'lucide-react';
import {
  findings,
  inventoryServices,
  libraryVulnerabilities,
  repositories,
  runtimeCodeVulnerabilities,
  triage,
  vulnKey,
  type CsFilter,
  type FindingKind,
  type LibraryVulnerability,
  type RuntimeCodeVulnerability,
  type StaticFinding,
} from '../../api/codeSecurity/client';
import { ErrorState, Loading } from '../../components/ui/AsyncState';
import { Select } from '../../components/ui/Dropdown';
import { SubTabs, VULN_TYPES } from './CodeSecurityLayout';
import { Drawer, fmtDate, Onboarding, PackageChip, Pill, SeverityBadge, SEVERITIES, shortRepo, timeAgo } from './common';

type VulnType = (typeof VULN_TYPES)[number]['id'];
const KIND: Partial<Record<VulnType, FindingKind>> = { 'static-code': 'sast', secrets: 'secret', iac: 'iac' };
const ALL = '';

const ONBOARDING: Record<VulnType, { icon: typeof Code2; product: string; title: string; text: string }> = {
  libraries: {
    icon: ShieldCheck,
    product: 'Software Composition Analysis (SCA)',
    title: 'Find vulnerable open source libraries',
    text: 'Libraries come from your services at runtime (the Datadog tracers report what they load) and from your repositories (datadog-ci sbom upload), matched against public advisories.',
  },
  'static-code': {
    icon: Code2,
    product: 'Static Code Analysis (SAST)',
    title: 'Find and fix vulnerabilities in first-party code',
    text: 'Upload the SARIF report of any static analyzer from CI with datadog-ci sarif upload; findings stay current with every scan of the branch, and pull request scans show what a change would introduce.',
  },
  'runtime-code': {
    icon: ScanEye,
    product: 'Runtime Code Analysis (IAST)',
    title: 'Find vulnerabilities in your running code',
    text: 'With IAST enabled (DD_IAST_ENABLED=true), Datadog tracers report SQL injection, SSRF, path traversal and other flaws on the requests that exercise them, down to the file and line.',
  },
  secrets: {
    icon: KeyRound,
    product: 'Secret Scanning',
    title: 'Detect and prioritize leaked secrets',
    text: 'Upload secret scanner results (the Datadog static analyzer, Gitleaks, TruffleHog…) as SARIF from CI to find credentials committed in your repositories.',
  },
  iac: {
    icon: Layers,
    product: 'Infrastructure as Code (IaC) Security',
    title: 'Catch misconfigurations before they reach production',
    text: 'Upload the SARIF report of an IaC scanner (Checkov, KICS, tfsec, Trivy…) from CI to see misconfigurations of your Terraform, Kubernetes and CloudFormation files.',
  },
};

function useOptions() {
  const repos = useQuery({ queryKey: ['code-security', 'repositories'], queryFn: ({ signal }) => repositories(signal) });
  const services = useQuery({ queryKey: ['code-security', 'inventory-services'], queryFn: ({ signal }) => inventoryServices(signal) });
  return {
    repos: [{ value: ALL, label: 'All' }, ...(repos.data ?? []).map((r) => ({ value: r.repository, label: shortRepo(r.repository) }))],
    services: [{ value: ALL, label: 'All' }, ...(services.data ?? []).map((s) => ({ value: s.service, label: s.service }))],
  };
}

const STATUSES = [
  { value: 'open', label: 'Open' },
  { value: 'muted', label: 'Muted' },
  { value: 'resolved', label: 'Resolved' },
  { value: 'all', label: 'All' },
];

/** Code Security → Vulnerabilities: one list per detection type. */
export function VulnerabilitiesPage() {
  const { type = 'libraries' } = useParams();
  const navigate = useNavigate();
  const [params, setParams] = useSearchParams();
  const vt = (VULN_TYPES.find((v) => v.id === type)?.id ?? 'libraries') as VulnType;
  const filter: CsFilter = {
    query: params.get('q') ?? '',
    severity: params.get('severity') ? [params.get('severity')!] : [],
    repository: params.get('repository') ?? '',
    service: params.get('service') ?? '',
    status: params.get('status') ?? 'open',
    scope: params.get('scope') ?? '',
    offset: Number(params.get('offset') ?? 0),
    limit: 25,
  };
  const set = (k: string, v: string) =>
    setParams(
      (p) => {
        const n = new URLSearchParams(p);
        if (v) n.set(k, v);
        else n.delete(k);
        if (k !== 'offset') n.delete('offset');
        return n;
      },
      { replace: true },
    );
  const opts = useOptions();
  const hasFilters = ['q', 'severity', 'repository', 'service', 'scope'].some((k) => params.get(k)) || filter.status !== 'open';
  return (
    <div className="cs-vulns">
      <SubTabs items={VULN_TYPES} active={vt} onPick={(id) => navigate(`/code-security/vulnerabilities/${id}`)} />
      <div className="cs-filters">
        <label className="apm-search cs-search">
          <Search size={14} />
          <input
            placeholder="Search"
            defaultValue={filter.query}
            key={vt}
            onKeyDown={(e) => e.key === 'Enter' && set('q', (e.target as HTMLInputElement).value)}
            onBlur={(e) => set('q', e.target.value)}
          />
        </label>
        <Select
          label="Severity"
          value={filter.severity?.[0] ?? ALL}
          onChange={(v) => set('severity', v)}
          options={[{ value: ALL, label: 'All' }, ...SEVERITIES.map((s) => ({ value: s, label: s[0].toUpperCase() + s.slice(1) }))]}
        />
        {vt !== 'runtime-code' && <Select label="Repository" value={filter.repository ?? ALL} onChange={(v) => set('repository', v)} options={opts.repos} />}
        {(vt === 'libraries' || vt === 'runtime-code') && <Select label="Service" value={filter.service ?? ALL} onChange={(v) => set('service', v)} options={opts.services} />}
        <Select
          label="Status"
          value={filter.status ?? 'open'}
          onChange={(v) => set('status', v === 'open' ? '' : v)}
          options={vt === 'libraries' || vt === 'runtime-code' ? STATUSES.filter((s) => s.value !== 'resolved') : STATUSES}
        />
        {vt === 'libraries' && (
          <Select
            label="Library Scope"
            value={filter.scope ?? ALL}
            onChange={(v) => set('scope', v)}
            options={[
              { value: ALL, label: 'All' },
              { value: 'production', label: 'Production' },
            ]}
          />
        )}
        {hasFilters && (
          <button className="link-btn" onClick={() => setParams({}, { replace: true })}>
            <X size={14} /> Clear
          </button>
        )}
      </div>
      {vt === 'libraries' ? (
        <LibraryList filter={filter} onPage={(o) => set('offset', String(o))} />
      ) : vt === 'runtime-code' ? (
        <RuntimeList filter={filter} onPage={(o) => set('offset', String(o))} />
      ) : (
        <FindingList kind={KIND[vt]!} vt={vt} filter={filter} onPage={(o) => set('offset', String(o))} />
      )}
    </div>
  );
}

function Pager({ total, offset, onPage }: { total: number; offset: number; onPage: (o: number) => void }) {
  if (total <= 25) return null;
  return (
    <div className="cs-pager">
      <button className="btn btn-sm" disabled={offset === 0} onClick={() => onPage(Math.max(0, offset - 25))}>
        Previous
      </button>
      <button className="btn btn-sm" disabled={offset + 25 >= total} onClick={() => onPage(offset + 25)}>
        Next
      </button>
    </div>
  );
}

function Empty({ vt, filtered }: { vt: VulnType; filtered: boolean }) {
  if (filtered) return <div className="cs-empty-row">No results match these filters.</div>;
  const o = ONBOARDING[vt];
  return (
    <Onboarding icon={<o.icon size={22} />} product={o.product} title={o.title} text={o.text}>
      <Link className="btn btn-primary" to="/code-security/settings">
        Get Started <ArrowRight size={14} />
      </Link>
    </Onboarding>
  );
}

function useTriage() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: ({ key, status }: { key: string; status: 'open' | 'muted' }) => triage(key, status),
    onSuccess: () => qc.invalidateQueries({ queryKey: ['code-security'] }),
  });
}

/* ---------- Libraries (SCA) ---------- */

function LibraryList({ filter, onPage }: { filter: CsFilter; onPage: (o: number) => void }) {
  const q = useQuery({ queryKey: ['code-security', 'libraries', filter], queryFn: ({ signal }) => libraryVulnerabilities(filter, signal), placeholderData: keepPreviousData });
  const [view, setView] = useState<'vulnerability' | 'finding'>('vulnerability');
  const [open, setOpen] = useState<LibraryVulnerability | null>(null);
  if (q.isPending) return <Loading />;
  if (q.isError) return <ErrorState error={q.error} onRetry={() => q.refetch()} />;
  const filtered = !!(filter.query || filter.severity?.length || filter.repository || filter.service || filter.scope) || filter.status !== 'open';
  if (!q.data.total) return <Empty vt="libraries" filtered={filtered} />;
  const rows = view === 'vulnerability' ? q.data.vulnerabilities.map((v) => ({ v, o: null })) : q.data.vulnerabilities.flatMap((v) => v.occurrences.map((o) => ({ v, o })));
  return (
    <div className="cs-list">
      <div className="cs-list-head">
        <div className="apm-seg">
          <span className="cs-seg-label">View as</span>
          <button className={view === 'vulnerability' ? 'active' : ''} onClick={() => setView('vulnerability')}>
            Vulnerability
          </button>
          <button className={view === 'finding' ? 'active' : ''} onClick={() => setView('finding')}>
            Finding
          </button>
        </div>
        <span>
          <b>
            {filter.offset! + 1}–{filter.offset! + q.data.vulnerabilities.length}
          </b>{' '}
          of <b>{q.data.total}</b> unique vulnerabilities
        </span>
        <span className="cs-sep" />
        <span>
          <b>{q.data.findings}</b> findings
        </span>
      </div>
      <table className="cs-table cs-vuln-table">
        <thead>
          <tr>
            <th>SEVERITY</th>
            <th>VULNERABILITY</th>
            <th>DETECTED IN</th>
            <th>STATUS</th>
          </tr>
        </thead>
        <tbody>
          {rows.map(({ v, o }) => (
            <tr key={`${v.key}|${o ? `${o.source}${o.service}${o.repository}${o.env}` : ''}`} className="cs-click" onClick={() => setOpen(v)}>
              <td>
                <SeverityBadge severity={v.severity} score={v.score} />
              </td>
              <td>
                <div className="cs-vuln-title">{v.title}</div>
                <div className="cs-vuln-meta">
                  <span className="cs-chip cs-mono">{v.cve ?? v.advisoryId}</span>
                  <PackageChip language={v.language} name={v.package} version={v.version} />
                </div>
                <div className="cs-vuln-meta">
                  {v.inProduction && (
                    <Pill icon={<Rocket size={12} />} tone="orange">
                      Service In Production
                    </Pill>
                  )}
                  {v.fixed && (
                    <Pill icon={<Wrench size={12} />} tone="blue">
                      Fix in {v.fixed}
                    </Pill>
                  )}
                </div>
              </td>
              <td>
                {o ? (
                  <span className="cs-chip">{o.source === 'runtime' ? `${o.service} · ${o.env || 'no env'}` : shortRepo(o.repository)}</span>
                ) : (
                  <div className="cs-detected">
                    <span className="cs-chip">
                      <GitBranch size={13} /> {v.repositories.length} {v.repositories.length === 1 ? 'Repository' : 'Repositories'}
                    </span>
                    <span className="cs-chip">
                      <Globe size={13} /> {v.services.length} {v.services.length === 1 ? 'Service' : 'Services'}
                    </span>
                  </div>
                )}
              </td>
              <td className="muted">{v.status === 'muted' ? 'Muted' : 'Open'}</td>
            </tr>
          ))}
        </tbody>
      </table>
      <Pager total={q.data.total} offset={filter.offset ?? 0} onPage={onPage} />
      {open && <LibraryDrawer v={open} onClose={() => setOpen(null)} />}
    </div>
  );
}

function LibraryDrawer({ v, onClose }: { v: LibraryVulnerability; onClose: () => void }) {
  const t = useTriage();
  const muted = v.status === 'muted';
  const [tab, setTab] = useState<'services' | 'repositories'>(v.services.length ? 'services' : 'repositories');
  return (
    <Drawer onClose={onClose} wide>
      <div className="cs-drawer-head">
        <SeverityBadge severity={v.severity} score={v.score} />
        <h2>{v.title}</h2>
      </div>
      <div className="cs-vuln-meta">
        <PackageChip language={v.language} name={v.package} version={v.version} />
        {v.inProduction && (
          <Pill icon={<Rocket size={12} />} tone="orange">
            Service In Production
          </Pill>
        )}
      </div>
      <div className="cs-detail">
        <div className="cs-detail-main">
          <section className="cs-card">
            <h4>DESCRIPTION</h4>
            <p>
              <Link to={`/code-security/detection-coverage/cves?advisory=${encodeURIComponent(v.advisoryId)}`}>{v.advisoryId}</Link> affects {v.package} {v.version}.
            </p>
            <dl className="cs-dl">
              <dt>DETECTED IN</dt>
              <dd>
                <span className="cs-chip">
                  <GitBranch size={13} /> {v.repositories.length ? `${v.repositories.length} repositories` : 'No source code detection'}
                </span>
                <span className="cs-chip">
                  <Globe size={13} /> {v.services.length} services
                </span>
              </dd>
              <dt>FIRST DETECTION</dt>
              <dd>
                <span className="cs-chip">{timeAgo(v.firstSeen)}</span> {fmtDate(v.firstSeen)}
              </dd>
              <dt>TAGS</dt>
              <dd>
                {[...v.aliases, ...v.cwes].map((x) => (
                  <span key={x} className="cs-chip cs-mono">
                    {x}
                  </span>
                ))}
              </dd>
            </dl>
          </section>
          <div className="cs-subtabs cs-subtabs-sm">
            <button className={tab === 'services' ? 'active' : ''} onClick={() => setTab('services')}>
              Impacted Services ({v.services.length})
            </button>
            <button className={tab === 'repositories' ? 'active' : ''} onClick={() => setTab('repositories')}>
              Repositories ({v.repositories.length})
            </button>
          </div>
          <table className="cs-table">
            <thead>
              <tr>
                <th>{tab === 'services' ? 'SERVICE' : 'REPOSITORY'}</th>
                <th>{tab === 'services' ? 'ENV' : 'FILE'}</th>
                <th>LAST SEEN</th>
              </tr>
            </thead>
            <tbody>
              {v.occurrences
                .filter((o) => (tab === 'services' ? o.source === 'runtime' : o.source === 'static'))
                .map((o) => (
                  <tr key={`${o.service}${o.repository}${o.env}`}>
                    <td>{tab === 'services' ? <Link to={`/apm/services/${encodeURIComponent(o.service)}`}>{o.service}</Link> : shortRepo(o.repository)}</td>
                    <td className="cs-mono">{tab === 'services' ? o.env || '—' : o.file || '—'}</td>
                    <td className="muted">{timeAgo(o.lastSeen)}</td>
                  </tr>
                ))}
            </tbody>
          </table>
        </div>
        <NextSteps
          remediation={v.fixed ? `Upgrade ${v.package} to ${v.fixed} or later.` : 'No fixed version is published for this advisory yet.'}
          muted={muted}
          busy={t.isPending}
          onToggle={() => t.mutate({ key: vulnKey(v), status: muted ? 'open' : 'muted' })}
        />
      </div>
    </Drawer>
  );
}

function NextSteps({ remediation, muted, busy, onToggle, extra }: { remediation: string; muted: boolean; busy: boolean; onToggle: () => void; extra?: React.ReactNode }) {
  return (
    <aside className="cs-next">
      <h4>NEXT STEPS</h4>
      <div className="cs-next-body">
        <b>Triage</b>
        <div className={`cs-next-state${muted ? ' muted' : ''}`}>{muted ? 'MUTED' : 'OPEN'}</div>
        <button className="btn" disabled={busy} onClick={onToggle}>
          {muted ? <Bell size={14} /> : <BellOff size={14} />} {muted ? 'Unmute' : 'Mute'}
        </button>
        <b>Remediation</b>
        <p>{remediation}</p>
        {extra}
        <b>More actions</b>
        <Link className="btn" to="/incidents?new=1">
          Declare Incident
        </Link>
      </div>
    </aside>
  );
}

/* ---------- Static findings (SAST, Secrets, IaC) ---------- */

function FindingList({ kind, vt, filter, onPage }: { kind: FindingKind; vt: VulnType; filter: CsFilter; onPage: (o: number) => void }) {
  const q = useQuery({ queryKey: ['code-security', 'findings', kind, filter], queryFn: ({ signal }) => findings(kind, filter, signal), placeholderData: keepPreviousData });
  const [open, setOpen] = useState<StaticFinding | null>(null);
  if (q.isPending) return <Loading />;
  if (q.isError) return <ErrorState error={q.error} onRetry={() => q.refetch()} />;
  const filtered = !!(filter.query || filter.severity?.length || filter.repository) || filter.status !== 'open';
  if (!q.data.total) return <Empty vt={vt} filtered={filtered} />;
  return (
    <div className="cs-list">
      <div className="cs-list-head">
        <span>
          <b>
            {filter.offset! + 1}–{filter.offset! + q.data.findings.length}
          </b>{' '}
          of <b>{q.data.total}</b> findings
        </span>
      </div>
      <table className="cs-table cs-vuln-table">
        <thead>
          <tr>
            <th>SEVERITY</th>
            <th>{kind === 'secret' ? 'SECRET' : kind === 'iac' ? 'MISCONFIGURATION' : 'VULNERABILITY'}</th>
            <th>LOCATION</th>
            <th>LAST DETECTED</th>
          </tr>
        </thead>
        <tbody>
          {q.data.findings.map((f) => (
            <tr key={f.id} className="cs-click" onClick={() => setOpen(f)}>
              <td>
                <SeverityBadge severity={f.severity} />
              </td>
              <td>
                <div className="cs-vuln-title">{f.ruleName}</div>
                <div className="cs-vuln-meta">
                  <span className="cs-chip cs-mono">{f.ruleId}</span>
                  {f.cwes.map((c) => (
                    <span key={c} className="cs-chip">
                      {c}
                    </span>
                  ))}
                  <span className="muted">{f.tool}</span>
                </div>
              </td>
              <td>
                <div>{shortRepo(f.repository)}</div>
                <div className="cs-mono muted">
                  {f.file}:{f.line}
                </div>
              </td>
              <td className="muted">{f.status === 'resolved' ? `Resolved ${timeAgo(f.resolvedAt)}` : timeAgo(f.lastSeen)}</td>
            </tr>
          ))}
        </tbody>
      </table>
      <Pager total={q.data.total} offset={filter.offset ?? 0} onPage={onPage} />
      {open && <FindingDrawer f={open} onClose={() => setOpen(null)} />}
    </div>
  );
}

function FindingDrawer({ f, onClose }: { f: StaticFinding; onClose: () => void }) {
  const t = useTriage();
  const muted = f.status === 'muted';
  return (
    <Drawer onClose={onClose} wide>
      <div className="cs-drawer-head">
        <SeverityBadge severity={f.severity} />
        <h2>{f.ruleName}</h2>
      </div>
      <div className="cs-vuln-meta">
        <span className="cs-chip cs-mono">{f.ruleId}</span>
        <span className="muted">{f.tool}</span>
      </div>
      <div className="cs-detail">
        <div className="cs-detail-main">
          <section className="cs-card">
            <h4>DESCRIPTION</h4>
            <p>{f.message}</p>
            {f.description && <p className="muted">{f.description}</p>}
            {f.snippet && (
              <pre className="cs-code">
                <span className="cs-code-line">{f.line}</span>
                {f.snippet}
              </pre>
            )}
            <dl className="cs-dl">
              <dt>DETECTED IN</dt>
              <dd>
                <span className="cs-chip">
                  <GitBranch size={13} /> {shortRepo(f.repository)}
                </span>
                <span className="cs-chip">{f.branch}</span>
                <span className="cs-chip cs-mono">{f.commitSha.slice(0, 7)}</span>
              </dd>
              <dt>FILE</dt>
              <dd className="cs-mono">
                {f.file}, Line {f.line}
              </dd>
              <dt>WHEN</dt>
              <dd>
                First detected {fmtDate(f.firstSeen)} · Last detected {fmtDate(f.lastSeen)}
              </dd>
              {f.cwes.length > 0 && (
                <>
                  <dt>TAGS</dt>
                  <dd>
                    {f.cwes.map((c) => (
                      <span key={c} className="cs-chip">
                        {c}
                      </span>
                    ))}
                  </dd>
                </>
              )}
            </dl>
          </section>
        </div>
        <NextSteps
          remediation={
            f.kind === 'secret' ? 'Revoke and rotate the credential, then remove it from the repository history.' : 'Fix the code at this location; the next scan of the branch resolves the finding.'
          }
          muted={muted}
          busy={t.isPending}
          onToggle={() => t.mutate({ key: `finding:${f.id}`, status: muted ? 'open' : 'muted' })}
          extra={
            f.helpUri ? (
              <a className="btn" href={f.helpUri} target="_blank" rel="noreferrer">
                Rule documentation <ExternalLink size={12} />
              </a>
            ) : undefined
          }
        />
      </div>
    </Drawer>
  );
}

/* ---------- Runtime code (IAST) ---------- */

function RuntimeList({ filter, onPage }: { filter: CsFilter; onPage: (o: number) => void }) {
  const q = useQuery({ queryKey: ['code-security', 'iast', filter], queryFn: ({ signal }) => runtimeCodeVulnerabilities(filter, signal), placeholderData: keepPreviousData });
  const [open, setOpen] = useState<RuntimeCodeVulnerability | null>(null);
  if (q.isPending) return <Loading />;
  if (q.isError) return <ErrorState error={q.error} onRetry={() => q.refetch()} />;
  const filtered = !!(filter.query || filter.severity?.length || filter.service) || filter.status !== 'open';
  if (!q.data.total) return <Empty vt="runtime-code" filtered={filtered} />;
  return (
    <div className="cs-list">
      <div className="cs-list-head">
        <span>
          <b>{q.data.total}</b> vulnerabilities detected in the past 7 days
        </span>
      </div>
      <table className="cs-table cs-vuln-table">
        <thead>
          <tr>
            <th>SEVERITY</th>
            <th>VULNERABILITY</th>
            <th>SERVICE</th>
            <th className="num">OCCURRENCES</th>
            <th>LAST DETECTED</th>
          </tr>
        </thead>
        <tbody>
          {q.data.vulnerabilities.map((v) => (
            <tr key={v.hash} className="cs-click" onClick={() => setOpen(v)}>
              <td>
                <SeverityBadge severity={v.severity} />
              </td>
              <td>
                <div className="cs-vuln-title">{v.title}</div>
                <div className="cs-vuln-meta cs-mono muted">
                  {v.path}
                  {v.method && `.${v.method}`}
                  {v.line ? `:${v.line}` : ''}
                </div>
              </td>
              <td>{v.services.join(', ')}</td>
              <td className="num">{v.count}</td>
              <td className="muted">{timeAgo(v.lastSeen)}</td>
            </tr>
          ))}
        </tbody>
      </table>
      <Pager total={q.data.total} offset={filter.offset ?? 0} onPage={onPage} />
      {open && <RuntimeDrawer v={open} onClose={() => setOpen(null)} />}
    </div>
  );
}

function RuntimeDrawer({ v, onClose }: { v: RuntimeCodeVulnerability; onClose: () => void }) {
  const t = useTriage();
  const muted = v.status === 'muted';
  return (
    <Drawer onClose={onClose} wide>
      <div className="cs-drawer-head">
        <SeverityBadge severity={v.severity} />
        <h2>{v.title}</h2>
      </div>
      <div className="cs-vuln-meta">
        {v.services.map((s) => (
          <span key={s} className="cs-chip">
            service:{s}
          </span>
        ))}
        {v.envs.map((e) => (
          <span key={e} className="cs-chip">
            env:{e}
          </span>
        ))}
        {v.cwe && <span className="cs-chip">{v.cwe}</span>}
      </div>
      <div className="cs-detail">
        <div className="cs-detail-main">
          <section className="cs-card">
            <h4>WHAT HAPPENED</h4>
            <p>
              The tracer saw untrusted input reach a sensitive operation ({v.type.replace(/_/g, ' ').toLowerCase()}) {v.count} times, last {timeAgo(v.lastSeen)}.
            </p>
            {v.evidence && <pre className="cs-code">{v.evidence}</pre>}
            <dl className="cs-dl">
              <dt>LOCATION</dt>
              <dd className="cs-mono">
                {v.path}
                {v.method && ` · ${v.method}`}
                {v.line ? ` · line ${v.line}` : ''}
              </dd>
              <dt>ENDPOINTS</dt>
              <dd>
                {v.resources.map((r) => (
                  <span key={r} className="cs-chip cs-mono">
                    {r}
                  </span>
                ))}
              </dd>
              <dt>WHEN</dt>
              <dd>
                First detected {fmtDate(v.firstSeen)} · Last detected {fmtDate(v.lastSeen)}
              </dd>
            </dl>
            <Link className="btn" to={`/apm/traces?query=${encodeURIComponent(`trace_id:${v.traceId}`)}`}>
              View sample trace
            </Link>
          </section>
        </div>
        <NextSteps
          remediation="Validate or parameterize the input at this location, then deploy: the vulnerability stops being reported."
          muted={muted}
          busy={t.isPending}
          onToggle={() => t.mutate({ key: `iast:${v.hash}`, status: muted ? 'open' : 'muted' })}
        />
      </div>
    </Drawer>
  );
}
