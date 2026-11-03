import { useMemo, useState } from 'react';
import { Link } from 'react-router-dom';
import type { EChartsOption } from 'echarts';
import { ChevronRight, Sparkles } from 'lucide-react';
import type { AssistReport } from '../../api/assist/client';
import { Chart } from '../../components/charts/Chart';
import { TimePicker } from '../../components/shell/PageHeader';
import { TimeProvider, useTime } from '../../components/shell/TimeContext';
import { ErrorState } from '../../components/ui/AsyncState';
import { Select } from '../../components/ui/Dropdown';
import { compact } from '../../lib/format';
import { palette } from '../../theme/chartTheme';
import { useAssistReport } from './queries';
import './assist.css';

const SURFACES = { chat: 'Assist Chat', panel: 'Ask Assist panel' } as Record<string, string>;

/** Assist → Reports: who uses Assist, how much, and where. */
export function AssistReportsPage() {
  return (
    <TimeProvider initial="1mo">
      <Reports />
    </TimeProvider>
  );
}

function bars(points: AssistReport['messagesPerDay'], name: string, color: string): EChartsOption {
  return {
    color: [color],
    grid: { left: 36, right: 12, top: 12, bottom: 24 },
    tooltip: { trigger: 'axis' },
    xAxis: { type: 'category', data: points.map((p) => new Date(p.at).toLocaleDateString('en-US', { month: 'short', day: 'numeric' })) },
    yAxis: { type: 'value', minInterval: 1 },
    series: [{ name, type: 'bar', data: points.map((p) => p.count), barMaxWidth: 18 }],
  };
}

function Reports() {
  const { range } = useTime();
  const [surface, setSurface] = useState('*');
  const minute = 60_000;
  const req = useMemo(
    () => ({ range: { from: Math.floor(range.from / minute) * minute, to: Math.ceil(range.to / minute) * minute }, surface: surface === '*' ? null : surface, userId: null }),
    [range.from, range.to, surface],
  );
  const q = useAssistReport(req);
  const r = q.data;
  const colors = palette();
  return (
    <div className="ai-reports">
      <header className="ai-reports-head">
        <div className="ai-crumbs">
          <Link to="/assist">
            <Sparkles size={16} /> Assist Chat
          </Link>
          <ChevronRight size={14} /> <b>Reports</b>
        </div>
        <div className="ai-row">
          <Select label="Surface" value={surface} onChange={setSurface} options={[{ value: '*', label: 'All' }, ...Object.entries(SURFACES).map(([value, label]) => ({ value, label }))]} />
          <TimePicker compact />
        </div>
      </header>
      {q.isError ? (
        <ErrorState error={q.error} onRetry={() => q.refetch()} />
      ) : (
        <div className="ai-report-grid">
          <Kpi label="Total Users" value={r?.totalUsers} />
          <Kpi label="Total Messages" value={r?.totalMessages} />
          <Kpi label="Total Conversations" value={r?.totalConversations} />
          <Kpi label="Tokens (in / out)" value={r ? `${compact(r.inputTokens)} / ${compact(r.outputTokens)}` : undefined} />
          <Panel title="Weekly Active Users" empty={!r?.weeklyActiveUsers.length}>
            {r && <Chart option={bars(r.weeklyActiveUsers, 'Users', colors[0])} height={200} />}
          </Panel>
          <Panel title="Messages by Surface" empty={!r?.messagesBySurface.length}>
            {r && (
              <ul className="ai-surfaces">
                {r.messagesBySurface.map((s) => (
                  <li key={s.surface}>
                    <span>{SURFACES[s.surface] ?? s.surface}</span>
                    <span className="ai-surface-bar">
                      <span style={{ width: `${(s.count / Math.max(1, r.totalMessages)) * 100}%` }} />
                    </span>
                    <b>{s.count}</b>
                  </li>
                ))}
              </ul>
            )}
          </Panel>
          <Panel title="Messages Over Time" empty={!r?.messagesPerDay.length}>
            {r && <Chart option={bars(r.messagesPerDay, 'Messages', colors[1])} height={200} />}
          </Panel>
          <Panel title="Conversations Over Time" empty={!r?.conversationsPerDay.some((p) => p.count)}>
            {r && <Chart option={bars(r.conversationsPerDay, 'Conversations', colors[2])} height={200} />}
          </Panel>
        </div>
      )}
    </div>
  );
}

function Kpi({ label, value }: { label: string; value: number | string | undefined }) {
  return (
    <section className="ai-card ai-kpi">
      <h3>{label}</h3>
      <b>{value ?? '–'}</b>
    </section>
  );
}

function Panel({ title, empty, children }: { title: string; empty: boolean; children: React.ReactNode }) {
  return (
    <section className="ai-card ai-panel-wide">
      <h3>{title}</h3>
      {empty ? <p className="muted ai-nodata">(No data)</p> : children}
    </section>
  );
}
