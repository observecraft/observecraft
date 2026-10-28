import { useMemo, useState } from 'react';
import { useNavigate } from 'react-router-dom';
import type { EChartsOption } from 'echarts';
import { Info } from 'lucide-react';
import type { FleetAgent, FleetAgentKind, FleetPage } from '../../api/fleet/types';
import { Chart } from '../../components/charts/Chart';
import { CodeBlock } from '../../components/code/CodeBlock';
import { ErrorState, Loading } from '../../components/ui/AsyncState';
import { statusColor, token } from '../../theme/chartTheme';
import { installGuide, kindLabel, outdatedByVersion, UPGRADE_GUIDES, type GuideKey, type VersionGroup } from './fleetModel';
import { KindIcon } from './fleetUi';
import { useFleet } from './queries';

/** Integrations → Fleet Automation → Upgrades: versions in the fleet, what is behind and how to upgrade it. */
export function UpgradesPage() {
  const page = useFleet();
  if (page.isError) return <ErrorState error={page.error} onRetry={() => page.refetch()} />;
  if (!page.data) return <Loading label="Loading agents…" />;
  if (!page.data.enabled) return <p className="fl-view muted">No agents reported in this time range.</p>;
  return <Upgrades page={page.data} />;
}

function Upgrades({ page }: { page: FleetPage }) {
  const groups = useMemo(() => outdatedByVersion(page.agents), [page.agents]);
  const outdated = page.agents.filter((a) => a.outdated);
  return (
    <div className="fl-view">
      <div className="fl-banner">
        <Info size={16} />
        <div>
          <strong>Upgrades happen where agents are deployed.</strong> ObserveCraft doesn’t upgrade agents remotely; it shows which ones are behind the newest version running in your fleet and how to
          move them forward with the tool that installed them.
        </div>
      </div>
      <div className="fl-cards">
        <LatestCard kind="datadog_agent" latest={page.latest.datadogAgent} agents={page.agents} />
        <LatestCard kind="otel_collector" latest={page.latest.otelCollector} agents={page.agents} />
      </div>
      <section className="fl-section">
        <h2 className="fl-h2">Version distribution</h2>
        <VersionChart page={page} />
      </section>
      <section className="fl-section">
        <h2 className="fl-h2">
          {outdated.length} outdated {outdated.length === 1 ? 'agent' : 'agents'}
        </h2>
        {groups.length === 0 ? <p className="muted">Every agent runs the newest version of its kind seen in the fleet.</p> : <OutdatedTable groups={groups} latest={page.latest} />}
      </section>
      <Guides agents={outdated.length ? outdated : page.agents} />
    </div>
  );
}

function LatestCard({ kind, latest, agents }: { kind: FleetAgentKind; latest: string | null | undefined; agents: FleetAgent[] }) {
  const mine = agents.filter((a) => a.kind === kind);
  const behind = mine.filter((a) => a.outdated).length;
  return (
    <div className="fl-card fl-card-static">
      <span className="fl-card-tile fl-tile-kind">
        <KindIcon kind={kind} size={22} />
      </span>
      <span className="fl-card-text">
        <strong>
          {kindLabel(kind)} {latest ?? <span className="muted">—</span>}
        </strong>
        <span className="muted">
          {mine.length === 0
            ? 'None in the fleet'
            : `Newest version seen · ${mine.length - behind} of ${mine.length} up to date${mine.some((a) => !a.version) ? ', some without a reported version' : ''}`}
        </span>
      </span>
    </div>
  );
}

function VersionChart({ page }: { page: FleetPage }) {
  const option = useMemo<EChartsOption>(() => {
    const rows = [...page.versions].reverse();
    return {
      grid: { left: 190, right: 32, top: 8, bottom: 24 },
      tooltip: { trigger: 'item', formatter: (p) => `${(p as { name: string }).name}: ${(p as { value: number }).value} agents` },
      xAxis: { type: 'value', minInterval: 1 },
      yAxis: { type: 'category', data: rows.map((v) => `${kindLabel(v.kind)} ${v.version ?? 'unknown'}`) },
      series: [
        {
          type: 'bar',
          barMaxWidth: 18,
          data: rows.map((v) => ({ value: v.count, itemStyle: { color: v.outdated ? statusColor('warn') : v.version ? statusColor('ok') : token('--status-nodata') } })),
          label: { show: true, position: 'right' },
        },
      ],
    };
  }, [page.versions]);
  return (
    <>
      <div className="fl-legend">
        <span>
          <span className="fl-dot fl-dot-ok" /> Newest
        </span>
        <span>
          <span className="fl-dot fl-dot-issues" /> Outdated
        </span>
      </div>
      <Chart option={option} height={Math.max(120, page.versions.length * 30 + 40)} />
    </>
  );
}

function OutdatedTable({ groups, latest }: { groups: VersionGroup<FleetAgent>[]; latest: FleetPage['latest'] }) {
  const navigate = useNavigate();
  return (
    <div className="fl-table-wrap">
      <table className="fl-table">
        <thead>
          <tr>
            <th>VERSION</th>
            <th>TYPE</th>
            <th>UPGRADE TO</th>
            <th className="num">AGENTS</th>
            <th>HOSTS</th>
            <th>INSTALLED WITH</th>
          </tr>
        </thead>
        <tbody>
          {groups.map((g) => (
            <tr key={`${g.kind} ${g.version}`}>
              <td>
                <span className="fl-version fl-version-outdated">{g.version}</span>
              </td>
              <td>
                <span className="fl-cell-icon">
                  <KindIcon kind={g.kind} /> {kindLabel(g.kind)}
                </span>
              </td>
              <td className="mono">{(g.kind === 'datadog_agent' ? latest.datadogAgent : latest.otelCollector) ?? '—'}</td>
              <td className="num">{g.agents.length}</td>
              <td>
                <span className="fl-chips">
                  {g.agents.slice(0, 4).map((a) => (
                    <button key={a.id} className="link-btn mono" onClick={() => navigate(`/fleet?agent=${encodeURIComponent(a.id)}`)}>
                      {a.hostname}
                    </button>
                  ))}
                  {g.agents.length > 4 && <span className="tag-chip">+{g.agents.length - 4}</span>}
                </span>
              </td>
              <td>{[...new Set(g.agents.map((a) => a.installMethod ?? 'not reported'))].join(', ')}</td>
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  );
}

function Guides({ agents }: { agents: FleetAgent[] }) {
  const keys = useMemo(() => {
    const counts = new Map<GuideKey, number>();
    for (const a of agents) {
      const k = installGuide(a.installMethod, a.kind);
      counts.set(k, (counts.get(k) ?? 0) + 1);
    }
    return [...counts].sort((x, y) => y[1] - x[1]);
  }, [agents]);
  const [picked, setPicked] = useState<GuideKey | null>(null);
  const current = picked && keys.some(([k]) => k === picked) ? picked : keys[0]?.[0];
  if (!current) return null;
  const guide = UPGRADE_GUIDES[current];
  return (
    <section className="fl-section fl-howto">
      <h2 className="fl-h2">How to upgrade</h2>
      <div className="fl-tabs" role="tablist">
        {keys.map(([k, n]) => (
          <button key={k} role="tab" aria-selected={k === current} className={`fl-tab${k === current ? ' active' : ''}`} onClick={() => setPicked(k)}>
            {UPGRADE_GUIDES[k].title} <span className="muted">{n}</span>
          </button>
        ))}
      </div>
      <p>{guide.intro}</p>
      <ol className="fl-steps">
        {guide.steps.map((s, i) => (
          <li key={i}>
            <p>{s.text}</p>
            {s.code && <CodeBlock code={s.code} />}
          </li>
        ))}
      </ol>
    </section>
  );
}
