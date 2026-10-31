import type { ReactNode } from 'react';
import { Link } from 'react-router-dom';
import type { EChartsOption } from 'echarts';
import { ArrowRight, Boxes, CheckCircle2, Cloud, Code2, FolderGit2, Radar, Server, ShieldAlert, Waypoints } from 'lucide-react';
import { Chart } from '../../components/charts/Chart';
import { ErrorState, Loading } from '../../components/ui/AsyncState';
import { compact } from '../../lib/format';
import { token } from '../../theme/chartTheme';
import { useSecurityOverview } from './queries';

/** Security → Overview: what is covered from development to runtime, and what each product found. */
export function SecurityOverviewPage() {
  const q = useSecurityOverview();
  if (q.isLoading) return <Loading />;
  if (q.isError) return <ErrorState error={q.error} onRetry={() => q.refetch()} />;
  const o = q.data!;
  const c = o.codeSecurity;
  const n = (on: boolean, v: number) => (on ? compact(v) : '—');
  const appSec: EChartsOption = {
    grid: { left: 24, right: 8, top: 8, bottom: 20 },
    xAxis: { type: 'time', axisLabel: { fontSize: 10 } },
    yAxis: { type: 'value', minInterval: 1, splitNumber: 1, axisLabel: { fontSize: 10 } },
    series: [{ type: 'bar', barMaxWidth: 10, itemStyle: { color: token('--chart-1') }, data: o.appSecSignals }],
  };
  return (
    <div className="sec-overview">
      <div className="sec-ov-col sec-ov-inventory">
        <div className="sec-ov-axis">
          <span>DEVELOPMENT</span>
          <span>RUNTIME</span>
        </div>
        <div className="sec-ov-boxes">
          <InventoryBox title="Repositories" icon={<FolderGit2 size={16} />}>
            {o.repositories ? (
              <p>
                <b>{compact(o.repositories)}</b> repositories analyzed
              </p>
            ) : (
              <p className="muted">No repository sends scans or SBOMs yet.</p>
            )}
          </InventoryBox>
          <InventoryBox title="Applications" icon={<Boxes size={16} />}>
            <p>
              <b>{compact(o.services)}</b> services
            </p>
            <p>
              <b>{compact(o.endpoints)}</b> endpoints
            </p>
          </InventoryBox>
          <InventoryBox title="Infrastructure" icon={<Server size={16} />}>
            <p>
              <b>{compact(o.hosts)}</b> hosts
            </p>
          </InventoryBox>
          <InventoryBox title="Cloud, SaaS, network, identity & endpoints" icon={<Cloud size={16} />}>
            {o.logSources ? (
              <p>
                <b>{o.logSources}</b> log sources sending security logs
              </p>
            ) : (
              <>
                <p className="muted">No log sources detected</p>
                <Link className="btn btn-primary" to="/security/siem/content-packs">
                  Configure log sources
                </Link>
              </>
            )}
          </InventoryBox>
        </div>
      </div>
      <div className="sec-ov-col sec-ov-products">
        <section className="sec-ov-product">
          <h3>
            <Code2 size={17} /> Code Security
          </h3>
          <Capability on={c.sast} label="Static Code Analysis (SAST)" />
          <Capability on={c.scaStatic} label="Software Composition Analysis (Static)" />
          <Capability on={c.iac} label="Infrastructure as Code (IaC)" />
          <Capability on={c.secrets} label="Secret Scanning" />
          <hr />
          <Capability on={c.iast} label="Runtime Code Analysis (IAST)" />
          <Capability on={c.scaRuntime} label="Software Composition Analysis (Runtime)" />
        </section>
        <section className="sec-ov-product">
          <h3>
            <Radar size={17} /> Cloud SIEM
          </h3>
          {o.siemPacksEnabled ? (
            <>
              <Capability on label={`${o.siemPacksEnabled} content packs on`} to="/security/siem/content-packs" />
              <Capability on={o.logSources > 0} label={o.logSources ? `${o.logSources} log sources sending logs` : 'Waiting for security logs'} to="/security/siem/content-packs" />
            </>
          ) : (
            <p className="sec-ov-pitch">
              Detect threats in your cloud audit, identity and access logs with detection rules for each log source.
              <Link className="btn" to="/security/siem/content-packs">
                Get Started
              </Link>
            </p>
          )}
        </section>
        <section className="sec-ov-product">
          <h3>
            <ShieldAlert size={17} /> App &amp; API Protection
          </h3>
          <p className="sec-ov-pitch">Application security rules can be saved; they aren&rsquo;t evaluated yet.</p>
        </section>
      </div>
      <div className="sec-ov-col sec-ov-results">
        <section className="sec-ov-result">
          <div className="sec-ov-row">
            <Metric value={n(c.sast, c.staticCodeVulnerabilities)} label="Static Code Vulnerabilities" to="/code-security/vulnerabilities/static-code" />
            <Metric value={n(c.iac, c.iacFindings)} label="Infrastructure as Code" to="/code-security/vulnerabilities/iac" />
            <Metric value={n(c.secrets, c.secretsFound)} label="Secrets" to="/code-security/vulnerabilities/secrets" />
          </div>
          <div className="sec-ov-row chain">
            <Metric value={n(c.scaStatic, c.staticLibraryVulnerabilities)} label="Static Library Vulnerabilities" to="/code-security/vulnerabilities/libraries" />
            <Metric value={n(c.scaStatic || c.scaRuntime, c.withExploit)} label="Exploited in the Wild" to="/security/feed" />
          </div>
        </section>
        <section className="sec-ov-result">
          <div className="sec-ov-row chain">
            <Metric value={n(c.iast, c.runtimeCodeVulnerabilities)} label="Runtime Code Vulnerabilities" to="/code-security/vulnerabilities/runtime-code" />
          </div>
          <div className="sec-ov-row chain">
            <Metric value={n(c.scaRuntime, c.runtimeLibraryVulnerabilities)} label="Runtime Library Vulnerabilities" to="/code-security/vulnerabilities/libraries" />
            <Metric value={n(c.scaRuntime, c.runtimeLibrariesInProduction)} label="In Production" to="/code-security/vulnerabilities/libraries" />
            <Metric value={n(c.scaRuntime, c.exposedToAttacks)} label="Exposed to Attacks" to="/security/signals" />
          </div>
        </section>
        <section className="sec-ov-result">
          <div className="sec-ov-row">
            <Metric
              value={o.siemPacksEnabled ? compact(o.importantSignals) : '—'}
              label="Important Signals"
              to="/security/siem/signals?q=status%3A%28high%20OR%20critical%29%20%40workflow.triage.state%3Aopen"
            />
          </div>
        </section>
        <section className="sec-ov-result">
          <div className="sec-ov-row">
            <Metric value={compact(o.appSecSignals.reduce((a, [, v]) => a + v, 0))} label="App & API Signals" badge="7d" to="/security/signals?q=%40workflow.rule.type%3Aapplication_security" />
            <div className="sec-ov-chart">
              <Chart option={appSec} height={70} />
            </div>
          </div>
        </section>
      </div>
    </div>
  );
}

function InventoryBox({ title, icon, children }: { title: string; icon: ReactNode; children: ReactNode }) {
  return (
    <section className="sec-ov-inv">
      <h4>
        {icon} {title}
      </h4>
      {children}
      <ArrowRight className="sec-ov-arrow" size={18} />
    </section>
  );
}

function Capability({ on, label, to }: { on: boolean; label: string; to?: string }) {
  return (
    <div className={`sec-ov-cap${on ? ' on' : ''}`}>
      {on ? <CheckCircle2 size={15} /> : <Waypoints size={15} />}
      <span>{label}</span>
      {to ? (
        <Link to={to}>View</Link>
      ) : (
        !on && (
          <Link className="sec-ov-enable" to="/code-security/settings">
            Enable
          </Link>
        )
      )}
    </div>
  );
}

function Metric({ value, label, to, badge }: { value: ReactNode; label: string; to: string; badge?: string }) {
  return (
    <Link className="sec-ov-metric" to={to}>
      <b>
        {value} {badge && <em>{badge}</em>}
      </b>
      <span>{label}</span>
    </Link>
  );
}
