import { useState } from 'react';
import { keepPreviousData, useQuery } from '@tanstack/react-query';
import { Link, useNavigate, useParams, useSearchParams } from 'react-router-dom';
import { ArrowRight, Download, ExternalLink, GitBranch, GitPullRequest, Globe, Layers, Lock, Search, ShieldCheck } from 'lucide-react';
import { advisories, advisory, inventoryLibraries, inventoryServices, pullRequests, repositories, rules, type CsFilter, type InventoryLibrary, type RuleSummary } from '../../api/codeSecurity/client';
import { ErrorState, Loading } from '../../components/ui/AsyncState';
import { Select } from '../../components/ui/Dropdown';
import { COVERAGE_TABS, SubTabs } from './CodeSecurityLayout';
import { Drawer, LangLogo, Onboarding, PackageChip, SeverityBadge, SEVERITIES, shortRepo, timeAgo } from './common';

const SEV_OPTIONS = [{ value: '', label: 'All' }, ...SEVERITIES.map((s) => ({ value: s, label: s[0].toUpperCase() + s.slice(1) }))];

function SearchBox({ value, onChange, placeholder }: { value: string; onChange: (v: string) => void; placeholder: string }) {
  return (
    <label className="apm-search cs-search">
      <Search size={14} />
      <input placeholder={placeholder} value={value} onChange={(e) => onChange(e.target.value)} />
    </label>
  );
}

/* ---------- Repositories ---------- */

export function RepositoriesPage() {
  const q = useQuery({ queryKey: ['code-security', 'repositories'], queryFn: ({ signal }) => repositories(signal) });
  const [text, setText] = useState('');
  if (q.isPending) return <Loading />;
  if (q.isError) return <ErrorState error={q.error} onRetry={() => q.refetch()} />;
  const rows = q.data.filter((r) => r.repository.toLowerCase().includes(text.toLowerCase()));
  return (
    <div className="cs-page">
      <SearchBox value={text} onChange={setText} placeholder="By repository" />
      <p className="cs-count-line">
        Showing <b>{rows.length}</b> of <b>{q.data.length}</b> repositories
      </p>
      <table className="cs-table cs-repo-table">
        <thead>
          <tr>
            <th />
            <th colSpan={2} className="cs-group-th">
              CODE
            </th>
            <th colSpan={2} className="cs-group-th">
              LIBRARIES
            </th>
            <th className="cs-group-th">SECRETS</th>
            <th className="cs-group-th">IAC</th>
            <th />
          </tr>
          <tr>
            <th>REPOSITORY</th>
            <th className="num">VULNERABILITIES</th>
            <th className="num">VIOLATIONS</th>
            <th className="num">VULNERABILITIES</th>
            <th className="num">LIBRARIES</th>
            <th className="num">DETECTED</th>
            <th className="num">VIOLATIONS</th>
            <th>LAST COMPLETED SCAN</th>
          </tr>
        </thead>
        <tbody>
          {rows.map((r) => (
            <tr key={r.repository}>
              <td>
                <span className="cs-repo">
                  <GitBranch size={14} /> {shortRepo(r.repository)}
                </span>
              </td>
              <td className="num">
                <Link to={`/code-security/vulnerabilities/static-code?repository=${encodeURIComponent(r.repository)}`}>{r.codeVulnerabilities}</Link>
              </td>
              <td className="num">{r.codeViolations}</td>
              <td className="num">
                <Link to={`/code-security/vulnerabilities/libraries?repository=${encodeURIComponent(r.repository)}`}>{r.libraryVulnerabilities}</Link>
              </td>
              <td className="num">{r.libraries}</td>
              <td className="num">
                <Link to={`/code-security/vulnerabilities/secrets?repository=${encodeURIComponent(r.repository)}`}>{r.secrets}</Link>
              </td>
              <td className="num">
                <Link to={`/code-security/vulnerabilities/iac?repository=${encodeURIComponent(r.repository)}`}>{r.iacViolations}</Link>
              </td>
              <td className="muted">
                {timeAgo(r.lastScan)} · {r.lastBranch} <span className="cs-mono">{r.lastCommit.slice(0, 7)}</span>
              </td>
            </tr>
          ))}
        </tbody>
      </table>
      {q.data.length === 0 && (
        <Onboarding
          icon={<GitBranch size={22} />}
          product="Repositories"
          title="No repositories analyzed yet"
          text="Repositories appear once CI uploads their SBOM (datadog-ci sbom upload) or a SARIF report (datadog-ci sarif upload)."
        >
          <Link className="btn btn-primary" to="/code-security/settings">
            Get Started <ArrowRight size={14} />
          </Link>
        </Onboarding>
      )}
    </div>
  );
}

/* ---------- Detection Coverage ---------- */

export function DetectionCoveragePage() {
  const { tab = 'cves' } = useParams();
  const navigate = useNavigate();
  return (
    <div className="cs-vulns">
      <SubTabs items={COVERAGE_TABS} active={tab} onPick={(id) => navigate(`/code-security/detection-coverage/${id}`)} />
      {tab === 'cves' ? <CveExplorer /> : <RulesList kind={tab === 'iac-rules' ? 'iac' : 'sast'} />}
    </div>
  );
}

function CveExplorer() {
  const [params, setParams] = useSearchParams();
  const [text, setText] = useState('');
  const [severity, setSeverity] = useState('');
  const [impact, setImpact] = useState('');
  const [ecosystem, setEcosystem] = useState('');
  const [offset, setOffset] = useState(0);
  const f = { query: text, severity: severity ? [severity] : [], impact, ecosystem, offset, limit: 25 };
  const q = useQuery({ queryKey: ['code-security', 'advisories', f], queryFn: ({ signal }) => advisories(f, signal), placeholderData: keepPreviousData });
  const openId = params.get('advisory');
  const close = () => setParams((p) => (p.delete('advisory'), p), { replace: true });
  return (
    <div className="cs-list">
      <div className="cs-filters">
        <SearchBox value={text} onChange={(v) => (setText(v), setOffset(0))} placeholder="Search by CVE ID, advisory ID, or package name" />
        <Select label="Severity" value={severity} onChange={(v) => (setSeverity(v), setOffset(0))} options={SEV_OPTIONS} />
        <Select
          label="Impact"
          value={impact}
          onChange={(v) => (setImpact(v), setOffset(0))}
          options={[
            { value: '', label: 'All' },
            { value: 'impacted', label: 'Impacted' },
            { value: 'not_impacted', label: 'Not impacted' },
          ]}
        />
        <Select
          label="Ecosystem"
          value={ecosystem}
          onChange={(v) => (setEcosystem(v), setOffset(0))}
          options={[{ value: '', label: 'All' }, ...['Maven', 'npm', 'PyPI', 'NuGet', 'Go', 'RubyGems', 'Packagist', 'crates.io'].map((e) => ({ value: e, label: e }))]}
        />
      </div>
      {q.isPending ? (
        <Loading />
      ) : q.isError ? (
        <ErrorState error={q.error} onRetry={() => q.refetch()} />
      ) : (
        <>
          <p className="cs-count-line">
            <b>{q.data.total}</b> advisories for the packages your services and repositories use
          </p>
          <table className="cs-table">
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
              {q.data.advisories.map((a) => (
                <tr key={a.id} className="cs-click" onClick={() => setParams((p) => (p.set('advisory', a.id), p), { replace: true })}>
                  <td>
                    <span className="cs-chip cs-mono">{a.cve ?? a.id}</span>
                  </td>
                  <td>
                    <div className="cs-vuln-title">{a.summary || a.id}</div>
                    <div className="cs-vuln-meta">
                      <span className="muted">packages:</span>
                      {a.packages.map((p) => (
                        <span key={p} className="cs-chip cs-mono">
                          {p}
                        </span>
                      ))}
                    </div>
                  </td>
                  <td>
                    <SeverityBadge severity={a.severity} score={a.score} />
                  </td>
                  <td className={a.impacted ? 'cs-impacted' : 'muted'}>{a.impacted ? 'Impacted' : 'Not impacted'}</td>
                  <td className="muted">{a.published ? new Date(a.published).toLocaleDateString('en-US', { month: 'short', day: 'numeric', year: 'numeric' }) : '—'}</td>
                </tr>
              ))}
            </tbody>
          </table>
          {q.data.total > 25 && (
            <div className="cs-pager">
              <button className="btn btn-sm" disabled={offset === 0} onClick={() => setOffset(offset - 25)}>
                Previous
              </button>
              <button className="btn btn-sm" disabled={offset + 25 >= q.data.total} onClick={() => setOffset(offset + 25)}>
                Next
              </button>
            </div>
          )}
          {q.data.total === 0 && <div className="cs-empty-row">No advisories match. Advisories are looked up for the packages Code Security has seen.</div>}
        </>
      )}
      {openId && <AdvisoryDrawer id={openId} onClose={close} />}
    </div>
  );
}

function AdvisoryDrawer({ id, onClose }: { id: string; onClose: () => void }) {
  const q = useQuery({ queryKey: ['code-security', 'advisory', id], queryFn: ({ signal }) => advisory(id, signal) });
  const [tab, setTab] = useState<'packages' | 'links'>('packages');
  return (
    <Drawer onClose={onClose} wide>
      {q.isPending ? (
        <Loading />
      ) : q.isError ? (
        <ErrorState error={q.error} />
      ) : (
        (() => {
          const a = q.data;
          const services = new Set(a.vulnerabilities.flatMap((v) => v.services));
          const repos = new Set(a.vulnerabilities.flatMap((v) => v.repositories));
          return (
            <>
              <div className="cs-vuln-meta">
                {a.aliases
                  .filter((x) => x.startsWith('CVE-'))
                  .map((x) => (
                    <span key={x}>
                      CVE: <span className="cs-chip cs-mono">{x}</span>
                    </span>
                  ))}
                <span>
                  Advisory ID: <span className="cs-chip cs-mono">{a.id}</span>
                </span>
              </div>
              <h2 className="cs-drawer-title">{a.summary || a.id}</h2>
              <div className="cs-vuln-meta">
                <SeverityBadge severity={a.severity} score={a.score} />
                <span className="muted">
                  Published {a.published ? `${new Date(a.published).toLocaleDateString('en-US', { month: 'short', day: 'numeric', year: 'numeric' })}, ${timeAgo(a.published)}` : '—'}
                </span>
              </div>
              <h3 className={services.size || repos.size ? 'cs-impacted' : ''}>Impacted resources</h3>
              <div className="cs-impact-cards">
                <div className={`cs-impact${repos.size ? ' hit' : ''}`}>
                  <GitBranch size={18} />
                  <span>
                    <b>{repos.size} repositories impacted</b>
                    <small>out of {a.repositoriesAnalyzed} analyzed</small>
                  </span>
                </div>
                <div className={`cs-impact${services.size ? ' hit' : ''}`}>
                  <Globe size={18} />
                  <span>
                    <b>{services.size} services impacted</b>
                    <small>out of {a.servicesAnalyzed} analyzed</small>
                  </span>
                </div>
              </div>
              <h3>Summary</h3>
              <p className="cs-pre">{a.details || a.summary}</p>
              {a.vector && <p className="cs-mono muted">{a.vector}</p>}
              <div className="cs-subtabs cs-subtabs-sm">
                <button className={tab === 'packages' ? 'active' : ''} onClick={() => setTab('packages')}>
                  Impacted Packages
                </button>
                <button className={tab === 'links' ? 'active' : ''} onClick={() => setTab('links')}>
                  Reference Links
                </button>
              </div>
              {tab === 'packages' ? (
                <table className="cs-table">
                  <thead>
                    <tr>
                      <th>PACKAGE</th>
                      <th>AFFECTED VERSIONS</th>
                      <th>FIXED IN</th>
                    </tr>
                  </thead>
                  <tbody>
                    {a.packages.map((p) => (
                      <tr key={`${p.ecosystem}${p.package}`}>
                        <td>
                          <span className="muted">{p.ecosystem}</span> <span className="cs-mono">{p.package}</span>
                        </td>
                        <td className="cs-mono">
                          {p.ranges.map((r) => `${r.introduced === '0' ? '' : `>= ${r.introduced} `}${r.fixed ? `< ${r.fixed}` : r.last_affected ? `<= ${r.last_affected}` : ''}`).join(', ') || '—'}
                        </td>
                        <td className="cs-mono">{p.fixed || '—'}</td>
                      </tr>
                    ))}
                  </tbody>
                </table>
              ) : (
                <table className="cs-table">
                  <thead>
                    <tr>
                      <th>TYPE</th>
                      <th>URL</th>
                    </tr>
                  </thead>
                  <tbody>
                    {a.refs.map((r) => (
                      <tr key={r.url}>
                        <td>{r.type}</td>
                        <td>
                          <a href={r.url} target="_blank" rel="noreferrer">
                            {r.url} <ExternalLink size={11} />
                          </a>
                        </td>
                      </tr>
                    ))}
                  </tbody>
                </table>
              )}
            </>
          );
        })()
      )}
    </Drawer>
  );
}

function RulesList({ kind }: { kind: 'sast' | 'iac' }) {
  const [text, setText] = useState('');
  const [severity, setSeverity] = useState('');
  const [language, setLanguage] = useState('');
  const f: CsFilter = { query: text, severity: severity ? [severity] : [], language };
  const q = useQuery({ queryKey: ['code-security', 'rules', kind, f], queryFn: ({ signal }) => rules(kind, f, signal), placeholderData: keepPreviousData });
  const [open, setOpen] = useState<RuleSummary | null>(null);
  return (
    <div className="cs-list">
      <div className="cs-filters">
        <SearchBox value={text} onChange={setText} placeholder="Find a rule name" />
        <Select label="Severity" value={severity} onChange={setSeverity} options={SEV_OPTIONS} />
        {kind === 'sast' && (
          <Select
            label="Language"
            value={language}
            onChange={setLanguage}
            options={[{ value: '', label: 'All' }, ...['java', 'python', 'nodejs', 'go', 'dotnet', 'ruby', 'php'].map((l) => ({ value: l, label: l }))]}
          />
        )}
      </div>
      {q.isPending ? (
        <Loading />
      ) : q.isError ? (
        <ErrorState error={q.error} onRetry={() => q.refetch()} />
      ) : q.data.length === 0 ? (
        <div className="cs-empty-row">No {kind === 'iac' ? 'IaC' : 'SAST'} rules yet: rules appear with the SARIF reports that use them.</div>
      ) : (
        <>
          <p className="cs-count-line">
            <b>{q.data.length}</b> rules
          </p>
          <table className="cs-table">
            <thead>
              <tr>
                <th>SUMMARY</th>
                <th>SEVERITY</th>
                <th>IMPACTED</th>
              </tr>
            </thead>
            <tbody>
              {q.data.map((r) => (
                <tr key={`${r.tool}${r.ruleId}`} className="cs-click" onClick={() => setOpen(r)}>
                  <td>
                    <div className="cs-vuln-title">
                      {kind === 'iac' ? <Layers size={14} /> : <LangLogo language={r.language} />} {r.name}
                    </div>
                    <div className="cs-vuln-meta">
                      <span className="cs-chip cs-mono">{r.ruleId}</span>
                      <span className="cs-chip">Tool: {r.tool}</span>
                      {r.cwes.map((c) => (
                        <span key={c} className="cs-chip">
                          CWE: {c}
                        </span>
                      ))}
                    </div>
                  </td>
                  <td>
                    <SeverityBadge severity={r.severity} />
                  </td>
                  <td>
                    <span className="cs-chip">
                      <GitBranch size={13} /> {r.repositories.length} Repositories
                    </span>
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        </>
      )}
      {open && (
        <Drawer onClose={() => setOpen(null)}>
          <span className="muted">{kind === 'iac' ? 'Infrastructure as Code Rule' : 'Static Analysis Rule'}</span>
          <h2 className="cs-drawer-title">{open.name}</h2>
          <div className="cs-vuln-meta">
            <SeverityBadge severity={open.severity} />
            <span className="cs-chip cs-mono">{open.ruleId}</span>
            {open.cwes.map((c) => (
              <span key={c} className="cs-chip">
                CWE: {c}
              </span>
            ))}
          </div>
          <h3>Summary</h3>
          <div className="cs-impact-cards">
            <div className={`cs-impact${open.openFindings ? ' hit' : ''}`}>
              <GitBranch size={18} />
              <span>
                <b>{open.openFindings} open findings</b>
                <small>in {open.repositories.length} repositories</small>
              </span>
            </div>
          </div>
          {open.description && (
            <>
              <h3>Description</h3>
              <p className="cs-pre">{open.description}</p>
            </>
          )}
          <Link className="btn" to={`/code-security/vulnerabilities/${kind === 'iac' ? 'iac' : 'static-code'}?q=${encodeURIComponent(open.ruleId)}`}>
            View findings <ArrowRight size={14} />
          </Link>
          {open.helpUri && (
            <a className="btn" href={open.helpUri} target="_blank" rel="noreferrer">
              Rule documentation <ExternalLink size={12} />
            </a>
          )}
        </Drawer>
      )}
    </div>
  );
}

/* ---------- Inventory ---------- */

export function InventoryPage() {
  const { tab = 'libraries' } = useParams();
  const navigate = useNavigate();
  return (
    <div className="cs-vulns">
      <SubTabs
        items={[
          { id: 'libraries', label: 'Libraries' },
          { id: 'services', label: 'Services' },
        ]}
        active={tab}
        onPick={(id) => navigate(`/code-security/inventory/${id}`)}
      />
      {tab === 'services' ? <ServicesInventory /> : <LibrariesInventory />}
    </div>
  );
}

function LibrariesInventory() {
  const [view, setView] = useState<'static' | 'runtime'>('static');
  const [text, setText] = useState('');
  const [language, setLanguage] = useState('');
  const [severity, setSeverity] = useState('');
  const f: CsFilter = { query: text, language, severity: severity ? [severity] : [], limit: 200 };
  const q = useQuery({ queryKey: ['code-security', 'inventory', view, f], queryFn: ({ signal }) => inventoryLibraries(view, f, signal), placeholderData: keepPreviousData });
  const exportSbom = (rows: InventoryLibrary[]) => {
    const doc = {
      bomFormat: 'CycloneDX',
      specVersion: '1.5',
      components: rows.map((l) => ({ type: 'library', name: l.name, version: l.version, licenses: l.licenses.map((id) => ({ license: { id } })) })),
    };
    const a = document.createElement('a');
    a.href = URL.createObjectURL(new Blob([JSON.stringify(doc, null, 2)], { type: 'application/json' }));
    a.download = `sbom-${view}.cdx.json`;
    a.click();
    URL.revokeObjectURL(a.href);
  };
  return (
    <div className="cs-inventory">
      <aside className="cs-facets">
        <div className="apm-seg">
          <span className="cs-seg-label">View</span>
          <button className={view === 'static' ? 'active' : ''} onClick={() => setView('static')}>
            Static
          </button>
          <button className={view === 'runtime' ? 'active' : ''} onClick={() => setView('runtime')}>
            Runtime
          </button>
        </div>
        <Select
          label="Language"
          value={language}
          onChange={setLanguage}
          options={[{ value: '', label: 'All' }, ...['java', 'nodejs', 'python', 'dotnet', 'go', 'ruby', 'php'].map((l) => ({ value: l, label: l }))]}
        />
        <Select label="Vulnerability Severity" value={severity} onChange={setSeverity} options={SEV_OPTIONS} />
      </aside>
      <div className="cs-list">
        <div className="cs-filters">
          <SearchBox value={text} onChange={setText} placeholder="Search for libraries" />
          <button className="btn" disabled={!q.data?.libraries.length} onClick={() => q.data && exportSbom(q.data.libraries)}>
            <Download size={14} /> Export SBOM
          </button>
        </div>
        {q.isPending ? (
          <Loading />
        ) : q.isError ? (
          <ErrorState error={q.error} onRetry={() => q.refetch()} />
        ) : q.data.total === 0 ? (
          <Onboarding
            icon={<ShieldCheck size={22} />}
            product="Inventory"
            title={view === 'static' ? 'Enable SCA in your code repositories' : 'No libraries reported by your services yet'}
            text={
              view === 'static'
                ? 'No libraries detected yet. Upload an SBOM from CI (datadog-ci sbom upload) to know which libraries your repositories declare before they reach production.'
                : 'Services report the libraries they load through the Datadog tracers’ telemetry, forwarded by the Agent.'
            }
          >
            <Link className="btn btn-primary" to="/code-security/settings">
              Get Started <ArrowRight size={14} />
            </Link>
          </Onboarding>
        ) : (
          <>
            <p className="cs-count-line">
              <b>{q.data.total}</b> libraries found
            </p>
            <table className="cs-table">
              <thead>
                <tr>
                  <th>LIBRARY</th>
                  <th>LICENSES</th>
                  <th className="num">UNIQUE VULNERABILITIES</th>
                  <th>{view === 'static' ? 'REPOSITORIES' : 'SERVICES'}</th>
                  <th>FILES</th>
                  <th>FIRST DETECTED</th>
                </tr>
              </thead>
              <tbody>
                {q.data.libraries.map((l) => (
                  <tr key={`${l.ecosystem}${l.name}${l.version}`}>
                    <td>
                      <PackageChip language={l.language} name={l.name} version={l.version} />
                    </td>
                    <td>{l.licenses.join(', ') || '—'}</td>
                    <td className="num">
                      {l.vulnerabilities ? <SeverityBadge severity={l.worstSeverity} /> : null} <b>{l.vulnerabilities}</b>
                    </td>
                    <td>{l.locations.map((x) => (view === 'static' ? shortRepo(x) : x)).join(', ')}</td>
                    <td className="cs-mono muted">{l.files.join(', ') || '—'}</td>
                    <td className="muted">{timeAgo(l.firstSeen)}</td>
                  </tr>
                ))}
              </tbody>
            </table>
          </>
        )}
      </div>
    </div>
  );
}

function ServicesInventory() {
  const q = useQuery({ queryKey: ['code-security', 'inventory-services'], queryFn: ({ signal }) => inventoryServices(signal) });
  if (q.isPending) return <Loading />;
  if (q.isError) return <ErrorState error={q.error} onRetry={() => q.refetch()} />;
  return (
    <div className="cs-list">
      <table className="cs-table">
        <thead>
          <tr>
            <th>SERVICE</th>
            <th>ENV</th>
            <th className="num">LIBRARIES</th>
            <th>LIBRARY VULNERABILITIES</th>
            <th className="num">RUNTIME CODE VULNERABILITIES</th>
            <th>LAST SEEN</th>
          </tr>
        </thead>
        <tbody>
          {q.data.map((s) => (
            <tr key={s.service}>
              <td>
                <span className="cs-pkg">
                  <LangLogo language={s.language} /> <Link to={`/apm/services/${encodeURIComponent(s.service)}`}>{s.service}</Link>
                </span>
              </td>
              <td>{s.envs.join(', ')}</td>
              <td className="num">{s.libraries}</td>
              <td>
                <span className="cs-count-sevs">
                  {(['critical', 'high', 'medium', 'low'] as const).map((k) => (
                    <span key={k} className={`cs-sev-text-${k}`}>
                      {s.vulnerabilities[k]} {k[0].toUpperCase()}
                    </span>
                  ))}
                </span>
              </td>
              <td className="num">{s.runtimeCodeVulnerabilities}</td>
              <td className="muted">{timeAgo(s.lastSeen)}</td>
            </tr>
          ))}
        </tbody>
      </table>
      {q.data.length === 0 && <div className="cs-empty-row">No service has reported its libraries yet.</div>}
    </div>
  );
}

/* ---------- PR Security ---------- */

export function PrSecurityPage() {
  const q = useQuery({ queryKey: ['code-security', 'pull-requests'], queryFn: ({ signal }) => pullRequests(signal) });
  if (q.isPending) return <Loading />;
  if (q.isError) return <ErrorState error={q.error} onRetry={() => q.refetch()} />;
  const all = q.data.flatMap((p) => p.findings.map((f) => ({ p, f })));
  const critical = all.filter(({ f }) => f.severity === 'critical' || f.severity === 'high').length;
  return (
    <div className="cs-page">
      <div className="cs-pr-cards">
        <div className="cs-pr-card red">
          Critical &amp; High <b>{critical}</b>
        </div>
        <div className="cs-pr-card">
          Open findings <b>{all.length}</b>
        </div>
        <div className="cs-pr-card">
          Pull requests scanned <b>{q.data.length}</b>
        </div>
      </div>
      <table className="cs-table">
        <thead>
          <tr>
            <th>SEVERITY</th>
            <th>WHEN</th>
            <th>WHAT</th>
            <th>WHERE</th>
          </tr>
        </thead>
        <tbody>
          {all.map(({ p, f }) => (
            <tr key={f.id}>
              <td>
                <SeverityBadge severity={f.severity} />
              </td>
              <td className="muted">{timeAgo(p.receivedAt)}</td>
              <td>
                <div className="cs-vuln-title">{f.ruleName}</div>
                <div className="cs-mono muted">
                  {f.file}:{f.line}
                </div>
              </td>
              <td>
                <span className="cs-chip">
                  <GitPullRequest size={13} /> {shortRepo(p.repository)} #{p.prNumber}
                </span>{' '}
                <span className="muted">
                  {p.branch} → {p.baseBranch || 'base'}
                </span>
              </td>
            </tr>
          ))}
        </tbody>
      </table>
      {all.length === 0 && (
        <div className="cs-empty-row">
          <Lock size={14} /> No findings introduced by pull requests. Run <code>datadog-ci sarif upload</code> in pull request pipelines to check changes before they merge.
        </div>
      )}
    </div>
  );
}
