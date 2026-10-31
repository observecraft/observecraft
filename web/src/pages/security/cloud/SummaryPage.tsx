import { useState, type ReactNode } from 'react';
import { Link } from 'react-router-dom';
import { AlertTriangle, ArrowRight, Box, Bug, ChevronDown, ChevronRight, Cpu, Database, FileWarning, Flame, HardDrive, Layers, Network, ShieldCheck, Users, Wrench } from 'lucide-react';
import { ErrorState, Loading } from '../../../components/ui/AsyncState';
import { compact } from '../../../lib/format';
import { useResearchFeed, useSecurityInbox } from '../queries';
import { useCloudSummary } from './queries';

const SECTIONS: [string, string][] = [
  ['prioritization', 'Prioritization'],
  ['coverage', 'Coverage'],
  ['inbox', 'Security Inbox'],
  ['findings', 'All findings'],
  ['inventory', 'Inventory'],
  ['feed', 'Research Feed'],
  ['knowledge', 'Knowledge'],
];

const CATEGORY_ICON: Record<string, ReactNode> = {
  Hosts: <Cpu size={16} />,
  Containers: <Box size={16} />,
  'Container Images': <Layers size={16} />,
  Orchestration: <Network size={16} />,
  Networking: <Network size={16} />,
  Storage: <HardDrive size={16} />,
  Databases: <Database size={16} />,
};

/** Cloud Security → Summary: vulnerability prioritization, coverage, inbox, findings, inventory and research. */
export function CloudSummaryPage() {
  const q = useCloudSummary();
  const inbox = useSecurityInbox();
  const feed = useResearchFeed();
  const [active, setActive] = useState('prioritization');
  const go = (id: string) => {
    setActive(id);
    document.getElementById(`csm-${id}`)?.scrollIntoView({ behavior: 'smooth', block: 'start' });
  };
  if (q.isLoading) return <Loading />;
  if (q.isError) return <ErrorState error={q.error} onRetry={() => q.refetch()} />;
  const s = q.data!;
  const v = s.vulnerabilities;
  const c = s.coverage;
  const hostCoverage = c.hosts ? Math.round((c.hostsScanned / c.hosts) * 100) : 0;
  const cloudInbox = (inbox.data?.findings ?? []).filter((f) => ['Misconfiguration', 'Infrastructure Vulnerability'].includes(f.findingType));
  const sev = (m: Record<string, number>) => (['critical', 'high', 'medium', 'low'] as const).map((k) => [k, m[k] ?? 0] as const);
  return (
    <div className="csm-summary">
      <aside className="csm-side">
        {SECTIONS.map(([id, label]) => (
          <button key={id} className={active === id ? 'active' : ''} onClick={() => go(id)}>
            {label}
          </button>
        ))}
      </aside>
      <div className="csm-main">
        <h2 className="sec-h">Cloud Security Summary</h2>
        <Section
          id="prioritization"
          title="Vulnerability Prioritization"
          sub="The vulnerabilities of your hosts and container images that matter most"
          link={['View in Vulnerability Explorer', '/security/cloud/vulnerabilities']}
        >
          <div className="csm-prio">
            {(
              [
                [<Bug size={15} key="i" />, 'Total CVEs', v.total, ''],
                [<Layers size={15} key="i" />, 'Running', v.running, '?running=1'],
                [<Flame size={15} key="i" />, 'Exploit available', v.exploitAvailable, '?exploit=1'],
                [<Wrench size={15} key="i" />, 'Fix available', v.fixAvailable, '?fix=1'],
                [<AlertTriangle size={15} key="i" />, 'High or Critical', v.highOrCritical, '?severity=high'],
                [<ShieldCheck size={15} key="i" />, 'High exploit risk', v.highExploitRisk, '?running=1&exploit=1'],
              ] as [ReactNode, string, number, string][]
            ).map(([icon, label, n, qs], _i, all) => (
              <Link key={label} className="csm-funnel" to={`/security/cloud/vulnerabilities${qs}`}>
                <span>
                  {icon} {label}
                </span>
                <b>{compact(n)}</b>
                <i style={{ height: `${all[0][2] ? Math.max(3, (n / (all[0][2] as number)) * 72) : 0}%` }} />
              </Link>
            ))}
          </div>
          {!v.total && <p className="muted csm-empty">No vulnerabilities: hosts and images haven&rsquo;t sent SBOMs, or none of their packages is affected.</p>}
        </Section>
        <Section id="coverage" title="Coverage" sub="What Cloud Security sees of your infrastructure" link={['Enhance Coverage', '/security/cloud/inventory']}>
          <div className="csm-cards">
            <div className="csm-card">
              <h5>CLOUD ACCOUNTS</h5>
              {c.accounts.length ? (
                <ul className="csm-list">
                  {c.accounts.map((a) => (
                    <li key={a} className="mono">
                      {a}
                    </li>
                  ))}
                </ul>
              ) : (
                <p className="muted">No cloud account tags on the catalog&rsquo;s resources yet.</p>
              )}
            </div>
            <div className="csm-card">
              <h5>HOSTS {hostCoverage < 50 && c.hosts > 0 && <span className="csm-badge bad">LOW COVERAGE</span>}</h5>
              <p className="csm-big">
                <b>{hostCoverage}%</b> coverage
              </p>
              <p className="muted">
                <b>{c.hostsScanned}</b> of <b>{c.hosts}</b> hosts send SBOMs
              </p>
            </div>
            <div className="csm-card">
              <h5>CONTAINER IMAGES &amp; KUBERNETES</h5>
              <p>
                <b>{c.images}</b> images scanned, <b>{c.imagesRunning}</b> running
              </p>
              <p>
                <b>{c.resourcesEvaluated}</b> resources evaluated in <b>{c.clusters}</b> cluster{c.clusters === 1 ? '' : 's'}
              </p>
            </div>
          </div>
        </Section>
        <Section id="inbox" title="Security Inbox" sub="The cloud findings to fix first" link={['View Issues in Security Inbox', '/security/cloud/inbox']}>
          {cloudInbox.length ? (
            <ul className="csm-inbox">
              {cloudInbox.slice(0, 5).map((f) => (
                <li key={f.key}>
                  <span className={`sec-sev sev-${f.severity}`}>{f.severity.toUpperCase()}</span>
                  <Link to={f.link}>{f.title}</Link>
                  <span className="muted">{f.resources.join(', ')}</span>
                </li>
              ))}
            </ul>
          ) : (
            <p className="muted csm-empty">You&rsquo;re all caught up: no critical or high cloud findings.</p>
          )}
        </Section>
        <Section id="findings" title="All Findings" sub="Every security finding of your infrastructure">
          <div className="csm-cards">
            <SeverityCard icon={<Wrench size={15} />} title="MISCONFIGURATIONS" counts={sev(s.misconfigurations)} to="/security/cloud/findings" />
            <SeverityCard icon={<Bug size={15} />} title="CLOUD VULNERABILITIES" counts={sev(s.vulnerabilitiesBySeverity)} to="/security/cloud/vulnerabilities" />
            <div className="csm-card">
              <h5>
                <Users size={15} /> IDENTITY RISKS
              </h5>
              <p className="muted">Identity risks need the cloud providers&rsquo; IAM data, which ObserveCraft doesn&rsquo;t collect yet.</p>
            </div>
          </div>
        </Section>
        <Section id="inventory" title="Inventory" sub="Your infrastructure as Cloud Security sees it" link={['Explore Resources', '/security/cloud/inventory']}>
          <div className="csm-inventory">
            {s.inventory.map((r) => (
              <Link key={r.category} className="csm-inv" to="/security/cloud/inventory">
                <span>
                  {CATEGORY_ICON[r.category] ?? <Box size={16} />} {r.category}
                </span>
                <b>{compact(r.count)}</b>
              </Link>
            ))}
          </div>
        </Section>
        <Section id="feed" title="Research Feed" sub="New and notable vulnerabilities and how they affect your infrastructure" link={['View All Research Feed Articles', '/security/feed']}>
          <div className="csm-feed">
            {(feed.data ?? []).slice(0, 3).map((i) => (
              <Link key={i.id} className="csm-feed-card" to="/security/feed">
                <div className={`sec-feed-tile ${i.articleType === 'exploited_vulnerability' ? 'exploited' : 'emerging'}`}>
                  <FileWarning size={26} />
                  <span>{i.articleType === 'exploited_vulnerability' ? 'Exploited Vulnerability' : 'Emerging Vulnerability'}</span>
                </div>
                <span className={i.impacted ? 'sec-impacted inline' : 'sec-not-detected'}>{i.impacted ? 'IMPACTED' : 'NOT DETECTED'}</span>
                <b>{i.title}</b>
                <p className="muted">{i.summary}</p>
              </Link>
            ))}
            {feed.isSuccess && !feed.data.length && <p className="muted">No articles yet.</p>}
          </div>
        </Section>
        <Section id="knowledge" title="Expand your knowledge" sub="How Cloud Security gets its data">
          <div className="csm-knowledge">
            <div>
              <b>Send SBOMs from the Datadog Agent</b>
              <p className="muted">
                Set <span className="mono">DD_SBOM_ENABLED</span>, <span className="mono">DD_SBOM_HOST_ENABLED</span> and <span className="mono">DD_SBOM_CONTAINER_IMAGE_ENABLED</span>, with{' '}
                <span className="mono">DD_SBOM_DD_URL</span> pointing at the intake.
              </p>
            </div>
            <div>
              <b>Kubernetes misconfigurations</b>
              <p className="muted">The Agent&rsquo;s orchestrator explorer sends the objects&rsquo; manifests; the rules check their pod templates and services.</p>
            </div>
            <div>
              <b>Advisories</b>
              <p className="muted">Packages are matched against OSV.dev advisories, and CVEs against CISA&rsquo;s known exploited catalog.</p>
              <Link to="/security/cloud/inventory/cves">
                Open the CVE Explorer <ArrowRight size={13} />
              </Link>
            </div>
          </div>
        </Section>
      </div>
    </div>
  );
}

function Section({ id, title, sub, link, children }: { id: string; title: string; sub: string; link?: [string, string]; children: ReactNode }) {
  const [open, setOpen] = useState(true);
  return (
    <section id={`csm-${id}`} className="csm-section">
      <header>
        <button className="link-btn" onClick={() => setOpen((o) => !o)} aria-label={open ? 'Collapse' : 'Expand'}>
          {open ? <ChevronDown size={18} /> : <ChevronRight size={18} />}
        </button>
        <h3>{title}</h3>
        <span className="muted">{sub}</span>
        {link && (
          <Link className="btn csm-section-link" to={link[1]}>
            {link[0]} <ArrowRight size={14} />
          </Link>
        )}
      </header>
      {open && children}
    </section>
  );
}

function SeverityCard({ icon, title, counts, to }: { icon: ReactNode; title: string; counts: readonly (readonly [string, number])[]; to: string }) {
  const total = counts.reduce((a, [, n]) => a + n, 0);
  return (
    <Link className="csm-card link" to={to}>
      <h5>
        {icon} {title}
      </h5>
      <p className="csm-big">
        <b>{compact(total)}</b> open
      </p>
      <div className="csm-sevs">
        {counts.map(([k, n]) => (
          <span key={k}>
            <i className={`sec-dot sev-${k}`} /> {n} {k}
          </span>
        ))}
      </div>
    </Link>
  );
}
