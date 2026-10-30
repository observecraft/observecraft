import { useMemo, useState } from 'react';
import { Link } from 'react-router-dom';
import { Copy, Search } from 'lucide-react';
import type { EChartsOption } from 'echarts';
import type { ScorecardOutcome } from '../../../api/scorecards/types';
import { Chart } from '../../../components/charts/Chart';
import { ErrorState, Loading } from '../../../components/ui/AsyncState';
import { Select } from '../../../components/ui/Dropdown';
import { palette, token } from '../../../theme/chartTheme';
import { useScorecards } from '../queries';
import { useScorecardsReport } from '../catalog/queries';
import { RangeSelect, useReports } from './ReportsLayout';
import { ReportSection } from './widgets';

const ALL = '*';

/** Passed share of the evaluated (not skipped) outcomes, in percent; null when none was evaluated. */
function score(outcomes: { state: string }[]): number | null {
  const evaluated = outcomes.filter((o) => o.state !== 'skip');
  return evaluated.length ? (100 * evaluated.filter((o) => o.state === 'pass').length) / evaluated.length : null;
}

const toneVar = (v: number) => (v >= 75 ? '--status-ok' : v >= 25 ? '--status-warn' : '--status-error');
const tone = (v: number) => `var(${toneVar(v)})`;

/** "51 %" with a bar filled to the score in its tone. */
function ScoreCell({ value }: { value: number | null }) {
  if (value === null) return <span className="pr-muted">—</span>;
  return (
    <span className="sp-score">
      <span style={{ color: tone(value) }}>{Math.round(value)} %</span>
      <span className="sp-bar">
        <i style={{ width: `${value}%`, background: tone(value) }} />
      </span>
    </span>
  );
}

/** A table with a search box, sortable on its first score column. */
function ScoreTable({ title, groups, columns }: { title: string; groups: string[]; columns: { label: string; value: (group: string) => number | null }[] }) {
  const [q, setQ] = useState('');
  const rows = groups.filter((g) => !q || g.toLowerCase().includes(q.toLowerCase())).sort((a, b) => (columns[0].value(b) ?? -1) - (columns[0].value(a) ?? -1));
  return (
    <div className="rw-card sp-table">
      <div className="rw-card-title">{title}</div>
      <label className="tv-filter sp-search">
        <Search size={14} />
        <input placeholder="Search" value={q} onChange={(e) => setQ(e.target.value)} />
      </label>
      <div className="sp-scroll">
        <table className="rw-legend">
          <thead>
            <tr>
              <th>OWNER</th>
              {columns.map((c) => (
                <th key={c.label} className="num">
                  {c.label.toUpperCase()}
                </th>
              ))}
            </tr>
          </thead>
          <tbody>
            {rows.map((g) => (
              <tr key={g}>
                <td>{g || <span className="pr-muted">No team</span>}</td>
                {columns.map((c) => (
                  <td key={c.label} className="num">
                    <ScoreCell value={c.value(g)} />
                  </td>
                ))}
              </tr>
            ))}
          </tbody>
        </table>
      </div>
    </div>
  );
}

/** Scores over time, one line per series, labelled at its end. */
function TrendChart({ title, label, series }: { title: string; label: string; series: { name: string; points: [number, number | null][] }[] }) {
  const colors = palette();
  const option: EChartsOption = {
    grid: { left: 8, right: 210, top: 16, bottom: 40, containLabel: true },
    tooltip: { trigger: 'axis', valueFormatter: (v) => (typeof v === 'number' ? `${v.toFixed(1)}` : '—') },
    legend: { bottom: 0, type: 'scroll', formatter: (n) => n },
    xAxis: { type: 'time' },
    yAxis: { type: 'value', min: 0, max: 100 },
    series: series.map((s, i) => ({
      name: s.name,
      type: 'line',
      showSymbol: false,
      connectNulls: true,
      color: colors[i % colors.length],
      data: s.points,
      endLabel: { show: series.length <= 8, formatter: '{a}', color: colors[i % colors.length], backgroundColor: token('--bg-muted'), padding: [2, 6], borderRadius: 3 },
    })),
  };
  return (
    <div className="rw-card sp-trend">
      <div className="rw-card-title">{title}</div>
      <span className="pr-muted sp-trend-label">{label}:</span>
      <Chart option={option} height="100%" />
    </div>
  );
}

/** Reports → Scorecards Performance: scores per team and scorecard, per rule and level, and how they evolved. */
export function ScorecardsPerformance() {
  const { range, teams: selectedTeams } = useReports();
  const overview = useScorecards();
  const [team, setTeam] = useState(selectedTeams[0] ?? ALL);
  const [service, setService] = useState(ALL);
  const [scorecard, setScorecard] = useState(ALL);
  const [rule, setRule] = useState(ALL);
  const [level, setLevel] = useState(ALL);
  const [groupBy, setGroupBy] = useState<'team' | 'entity'>('team');
  const report = useScorecardsReport({ range, teams: team === ALL ? [] : [team] });
  const data = overview.data;

  const model = useMemo(() => {
    if (!data?.setUp) return null;
    const teamOf = new Map(data.services.map((s) => [s.service, s.team]));
    const rules = data.rules.filter((r) => r.enabled && (scorecard === ALL || r.scorecardId === scorecard) && (rule === ALL || r.id === rule) && (level === ALL || String(r.level) === level));
    const ruleIds = new Set(rules.map((r) => r.id));
    const outcomes = data.outcomes.filter((o) => ruleIds.has(o.ruleId) && (team === ALL || teamOf.get(o.service) === team) && (service === ALL || o.service === service));
    const groupOf = (o: ScorecardOutcome) => (groupBy === 'team' ? (teamOf.get(o.service) ?? '') : o.service);
    const groups = [...new Set(outcomes.map(groupOf))];
    const scorecardOf = new Map(data.rules.map((r) => [r.id, r.scorecardId]));
    const scorecards = data.scorecards.filter((s) => rules.some((r) => r.scorecardId === s.id));
    return { rules, outcomes, groupOf, groups, scorecardOf, scorecards };
  }, [data, team, service, scorecard, rule, level, groupBy]);

  if (overview.isPending || report.isPending) return <Loading label="Loading the report…" />;
  if (overview.isError) return <ErrorState error={overview.error} onRetry={() => overview.refetch()} />;
  if (report.isError) return <ErrorState error={report.error} onRetry={() => report.refetch()} />;
  if (!data?.setUp || !model)
    return (
      <div className="pr-report">
        <h1 className="pr-title">Scorecards Performance</h1>
        <div className="pr-empty">
          <p>Scorecards aren't set up yet. Once they are, every evaluation records each service's results here.</p>
          <Link className="btn btn-primary" to="/portal/scorecards">
            Set up Scorecards
          </Link>
        </div>
      </div>
    );

  const { rules, outcomes, groupOf, groups, scorecardOf, scorecards } = model;
  const ofGroup = (g: string) => outcomes.filter((o) => groupOf(o) === g);
  const ruleIds = new Set(rules.map((r) => r.id));
  // History: the day's passes over the evaluated results, of the rules that pass the filters.
  const days = [...new Set(report.data.ruleDays.map((d) => d.day))].sort((a, b) => a - b);
  const history = (ids: Set<string>): [number, number | null][] =>
    days.map((day) => {
      const rows = report.data.ruleDays.filter((d) => d.day === day && ids.has(d.ruleId) && ruleIds.has(d.ruleId));
      const total = rows.reduce((n, r) => n + r.pass + r.fail, 0);
      return [day, total ? (100 * rows.reduce((n, r) => n + r.pass, 0)) / total : null];
    });
  const options = (values: { value: string; label: string }[]) => [{ value: ALL, label: '*' }, ...values];
  const owner = groupBy === 'team' ? 'Team' : 'Entity';

  return (
    <div className="pr-report">
      <div className="rw-filters">
        <Select label="team" value={team} options={options([...new Set(data.services.map((s) => s.team).filter(Boolean))].sort().map((t) => ({ value: t, label: t })))} onChange={setTeam} />
        <Select label="service" value={service} options={options(data.services.map((s) => ({ value: s.service, label: s.service })))} onChange={setService} />
        <Select label="scorecard_name" value={scorecard} options={options(data.scorecards.map((s) => ({ value: s.id, label: s.name })))} onChange={setScorecard} />
        <Select label="rule_name" value={rule} options={options(data.rules.map((r) => ({ value: r.id, label: r.name })))} onChange={setRule} />
        <Select label="rule_level" value={level} options={options(['1', '2', '3'].map((l) => ({ value: l, label: l })))} onChange={setLevel} />
        <span className="rw-inline">
          Group by
          <span className="btn-group">
            <button className={`btn btn-sm${groupBy === 'team' ? ' btn-active' : ''}`} onClick={() => setGroupBy('team')}>
              Team
            </button>
            <button className={`btn btn-sm${groupBy === 'entity' ? ' btn-active' : ''}`} onClick={() => setGroupBy('entity')}>
              Entity
            </button>
          </span>
        </span>
        <span className="rw-push">
          <RangeSelect />
        </span>
      </div>
      <div className="rw-title-row">
        <h1 className="pr-title">Scorecards Performance by {owner}</h1>
        <Link className="link-btn" to="/dashboards">
          <Copy size={14} /> Clone as Dashboard
        </Link>
      </div>

      <ReportSection id="by-scorecard" title="Performance by Scorecard" banner>
        <p className="sp-note">
          This data is based on your <Link to="/portal/scorecards">Scorecards</Link>, including default and custom rules. Rules evaluate the services in their scope; skipped results don't count.
        </p>
        <div className="sp-two">
          <ScoreTable title={`${owner} Leaderboard`} groups={groups} columns={[{ label: 'All Scorecards', value: (g) => score(ofGroup(g)) }]} />
          <ScoreTable
            title={`Performance by ${owner} for Scorecards`}
            groups={groups}
            columns={scorecards.map((sc) => ({ label: sc.name, value: (g) => score(ofGroup(g).filter((o) => scorecardOf.get(o.ruleId) === sc.id)) }))}
          />
        </div>
      </ReportSection>

      <ReportSection id="scorecard-trends" title="Historical Trends by Scorecard" banner>
        <TrendChart
          title="Historical Scorecard Performance"
          label="scorecard_name"
          series={scorecards.map((sc) => ({ name: sc.name, points: history(new Set(rules.filter((r) => r.scorecardId === sc.id).map((r) => r.id))) }))}
        />
      </ReportSection>

      <ReportSection id="by-rule" title="Performance by Rule" banner>
        <div className="sp-rules">
          <div className="rw-card sp-table">
            <div className="rw-card-title">Rule Performance</div>
            <div className="sp-scroll">
              <table className="rw-legend">
                <thead>
                  <tr>
                    <th>RULE_NAME</th>
                    <th className="num">SCORE</th>
                  </tr>
                </thead>
                <tbody>
                  {rules
                    .map((r) => ({ r, v: score(outcomes.filter((o) => o.ruleId === r.id)) }))
                    .sort((a, b) => (b.v ?? -1) - (a.v ?? -1))
                    .map(({ r, v }) => (
                      <tr key={r.id}>
                        <td>{r.name}</td>
                        <td className="num">
                          <ScoreCell value={v} />
                        </td>
                      </tr>
                    ))}
                </tbody>
              </table>
            </div>
          </div>
          {[1, 2, 3].map((l) => {
            const ids = new Set(rules.filter((r) => r.level === l).map((r) => r.id));
            const v = score(outcomes.filter((o) => ids.has(o.ruleId)));
            const points = history(ids);
            return (
              <div key={l} className="rw-card sp-level" style={{ background: v === null ? undefined : `color-mix(in srgb, ${tone(v)} 14%, var(--bg))` }}>
                <div className="rw-card-title">Level {l} Rules</div>
                <div className="sp-level-body">
                  {points.length > 1 && (
                    <div className="sp-level-bg">
                      <Chart
                        height="100%"
                        option={{
                          animation: false,
                          grid: { left: 0, right: 0, top: 0, bottom: 0 },
                          xAxis: { type: 'time', show: false },
                          yAxis: { type: 'value', show: false, min: 0, max: 100 },
                          series: [
                            {
                              type: 'line',
                              showSymbol: false,
                              silent: true,
                              areaStyle: { opacity: 0.35 },
                              color: token(v === null ? '--border-strong' : toneVar(v)),
                              lineStyle: { width: 1 },
                              data: points,
                            },
                          ],
                        }}
                      />
                    </div>
                  )}
                  <strong>{v === null ? '—' : `${Math.round(v)} %`}</strong>
                </div>
              </div>
            );
          })}
        </div>
        <div className="sp-rule-grid">
          {rules.map((r) => {
            const rows = groups
              .map((g) => ({ g, v: score(ofGroup(g).filter((o) => o.ruleId === r.id)) }))
              .filter((x) => x.v !== null)
              .sort((a, b) => b.v! - a.v!);
            return (
              <div key={r.id} className="sp-rule">
                <span className="sp-rule-name" title={r.name}>
                  {r.name}
                </span>
                <div className="sp-rule-list">
                  {rows.map(({ g, v }) => (
                    <div key={g} className="sp-rule-row">
                      <span className="num">{v!.toFixed(1)}</span>
                      <span className="sp-rule-bar" style={{ background: `linear-gradient(to right, color-mix(in srgb, ${tone(v!)} 35%, var(--bg)) ${v}%, transparent ${v}%)` }}>
                        {g || 'n/a'}
                      </span>
                    </div>
                  ))}
                </div>
              </div>
            );
          })}
        </div>
      </ReportSection>

      <ReportSection id="rule-trends" title="Historical Trends by Rule" banner>
        <TrendChart title="Historical Rule Performance" label="rule_name" series={rules.map((r) => ({ name: r.name, points: history(new Set([r.id])) }))} />
      </ReportSection>
    </div>
  );
}
