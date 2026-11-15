import { useState, type ReactNode } from 'react';
import { Link } from 'react-router-dom';
import type { EChartsOption } from 'echarts';
import { AlertTriangle, ArrowRight, Boxes, Bot, CheckCircle2, Cloud, CloudCog, Code2, Cpu, FolderGit2, Loader2, Radar, Server, ShieldAlert, Waypoints } from 'lucide-react';
import { Chart } from '../../components/charts/Chart';
import { ErrorState, Loading } from '../../components/ui/AsyncState';
import { compact } from '../../lib/format';
import { token } from '../../theme/chartTheme';
import { useSecurityOverview } from './queries';
import { useCloudSummary } from './cloud/queries';
import { useWpSummary } from './workload/overviewQueries';
import { useAiGuardSummary } from './aiGuard/queries';
import { useAppsecServices } from './appsec/queries';
import { cloudSecurityCard, workloadCard, type CapabilityState } from './overviewCards';

const WEEK = 7 * 86_400_000;

/** Security → Overview: what is covered from development to runtime, and what each product found. */
export function SecurityOverviewPage() {
  const q = useSecurityOverview();
  // The products' own summaries, over the last 7 days (fixed when the page opens).
  const [range] = useState(() => ({ from: Date.now() - WEEK, to: Date.now() }));
  const cloud = cloudSecurityCard(useCloudSummary());
  const wp = workloadCard(useWpSummary(range));
  const ai = useAiGuardSummary({ range }).data;
  const appsecServices = useAppsecServices().data;
  if (q.isLoading) return <Loading />;
  if (q.isError) return <ErrorState error={q.error} onRetry={() => q.refetch()} />;
  const o = q.data!;
  const c = o.codeSecurity;
  const aiOn = !!ai && ai.total > 0;
  const appsecOn = (appsecServices ?? []).filter((s) => s.protected || s.securityTraces > 0);
  const n = (on: boolean, v: number) => (on ? compact(v) : '—');
  // A product's number once its summary is read: '…' while it loads, '—' when the product isn't set up or failed.
  const of = (state: CapabilityState, v: number | undefined) => (state === 'pending' ? '…' : v === undefined ? '—' : compact(v));
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
          <Capability
            on={appsecOn.length > 0}
            label={appsecOn.length ? `${appsecOn.length} service${appsecOn.length === 1 ? '' : 's'} sending security traces` : 'Threat detection on security traces'}
            to={appsecOn.length ? '/security/appsec/inventory/services' : undefined}
            enable="/security/appsec/settings"
          />
          <Capability on={appsecOn.length > 0} label="Detection rules evaluated on attacks" to={appsecOn.length ? '/security/appsec/policies/rules' : undefined} enable="/security/appsec/settings" />
          <p className="sec-ov-note">In-app blocking needs Remote Configuration, which ObserveCraft doesn&rsquo;t serve yet: blocking rules are kept, not enforced.</p>
        </section>
        <section className="sec-ov-product">
          <h3>
            <CloudCog size={17} /> Cloud Security
          </h3>
          <Capability state={cloud.sbom} label="Host & container image vulnerabilities (SBOMs)" to="/security/cloud/vulnerabilities" enable="/security/cloud/settings" />
          <Capability state={cloud.posture} label="Misconfigurations (posture)" to="/security/cloud/findings" enable="/security/cloud/settings" />
          <Capability
            state={cloud.accounts}
            label={cloud.connectedAccounts ? `${cloud.connectedAccounts} cloud account${cloud.connectedAccounts === 1 ? '' : 's'} connected` : 'Cloud accounts'}
            to="/security/cloud"
            enable="/security/cloud/settings"
          />
        </section>
        <section className="sec-ov-product">
          <h3>
            <Cpu size={17} /> Workload Protection
          </h3>
          <Capability
            state={wp.runtime}
            label={wp.runtime === 'on' ? `${wp.hostsEnabled} of ${wp.hosts} hosts with runtime security` : 'Runtime security on Agents'}
            to="/security/workload-protection/inventory/coverage"
            enable="/security/workload-protection/setup"
          />
          <Capability
            state={wp.detectionRules}
            label={wp.rules === undefined ? 'Detection rules' : `${wp.rules} detection rule${wp.rules === 1 ? '' : 's'}`}
            to="/security/workload-protection/policies/detection-rules"
            enable="/security/workload-protection/setup"
          />
        </section>
        <section className="sec-ov-product">
          <h3>
            <Bot size={17} /> AI Guard
          </h3>
          <Capability
            on={aiOn}
            label={aiOn ? `${ai!.services.length} AI service${ai!.services.length === 1 ? '' : 's'} evaluated` : 'Prompt and tool call evaluation'}
            to={aiOn ? '/security/ai-guard' : undefined}
            enable="/security/ai-guard/settings"
          />
          <Capability on={!!ai?.blockingEnabled} label="Blocking unsafe prompts" to="/security/ai-guard/settings" />
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
        <section className="sec-ov-result">
          <div className="sec-ov-row">
            <Metric value={of(cloud.sbom, cloud.vulnerabilities)} label="Cloud Vulnerabilities" to="/security/cloud/vulnerabilities" />
            <Metric value={of(cloud.sbom, cloud.highExploitRisk)} label="High Exploit Risk" to="/security/cloud/vulnerabilities?running=1&exploit=1" />
            <Metric value={of(cloud.posture, cloud.misconfigurations)} label="Misconfigurations" to="/security/cloud/findings" />
          </div>
        </section>
        <section className="sec-ov-result">
          <div className="sec-ov-row">
            <Metric value={of(wp.runtime, wp.runtime === 'on' ? wp.signals : undefined)} label="Workload Signals" badge="7d" to="/security/workload-protection/signals" />
            <Metric value={aiOn ? compact(ai!.unsafeCount) : '—'} label="Unsafe AI Prompts" badge="7d" to="/security/ai-guard" />
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

/** A product capability: on (with a link to it), off (with a link to set it up), or its product's summary still loading or failed. */
function Capability({ on, state, label, to, enable = '/code-security/settings' }: { on?: boolean; state?: CapabilityState; label: string; to?: string; enable?: string }) {
  const s: CapabilityState = state ?? (on ? 'on' : 'off');
  const icon = { on: <CheckCircle2 size={15} />, off: <Waypoints size={15} />, pending: <Loader2 size={15} className="sec-ov-spin" />, error: <AlertTriangle size={15} /> }[s];
  return (
    <div className={`sec-ov-cap${s === 'on' ? ' on' : ''}`}>
      {icon}
      <span>{label}</span>
      {s === 'on' && to && <Link to={to}>View</Link>}
      {s === 'off' && (
        <Link className="sec-ov-enable" to={enable}>
          Enable
        </Link>
      )}
      {s === 'pending' && <span className="sec-ov-state">Checking…</span>}
      {s === 'error' && <span className="sec-ov-state">Couldn&rsquo;t load</span>}
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
