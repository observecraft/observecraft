// AI Credits Management (limits on Assist's use of the organization's model) and Usage (what each Assist
// application used, per member and day, with an estimated cost).
import { useEffect, useMemo, useState } from 'react';
import { Link } from 'react-router-dom';
import type { EChartsOption } from 'echarts';
import { Activity, Bot, CircleAlert, Code2, Coins, MessageSquare, MousePointerClick, ShieldCheck, Sparkles } from 'lucide-react';
import type { CreditSettings } from '../../api/assist/client';
import { Chart } from '../../components/charts/Chart';
import { TimePicker } from '../../components/shell/PageHeader';
import { TimeProvider, useTime } from '../../components/shell/TimeContext';
import { ErrorState, Loading } from '../../components/ui/AsyncState';
import { compact } from '../../lib/format';
import { palette } from '../../theme/chartTheme';
import { ProductHeader } from './RunView';
import { useCredits, useSaveCredits, useUsage } from './runsQueries';
import './assist.css';

const num = (v: string): number | null => (v.trim() === '' ? null : Number(v));
const usd = (v: number | null | undefined) => (v == null ? '—' : v < 0.01 && v > 0 ? '< $0.01' : `$${v.toFixed(2)}`);

export function CreditsPage() {
  const q = useCredits();
  const save = useSaveCredits();
  const [form, setForm] = useState<Record<keyof CreditSettings, string> | null>(null);
  useEffect(() => {
    if (q.data) {
      const s = q.data;
      setForm({
        monthlyLimit: s.monthlyLimit?.toString() ?? '',
        userDailyLimit: s.userDailyLimit?.toString() ?? '',
        inputPricePerMtok: s.inputPricePerMtok?.toString() ?? '',
        outputPricePerMtok: s.outputPricePerMtok?.toString() ?? '',
      });
    }
  }, [q.data]);
  if (q.isError) return <ErrorState error={q.error} onRetry={() => q.refetch()} />;
  if (!q.data || !form) return <Loading />;
  const c = q.data;
  const pct = c.monthlyLimit ? Math.min(100, (c.usedThisMonth / c.monthlyLimit) * 100) : 0;
  const field = (k: keyof CreditSettings, label: string, hint: string, step = '1') => (
    <label className="form-label ai-num">
      {label}
      <input className="settings-input" type="number" min={0} step={step} value={form[k]} placeholder="Unlimited" onChange={(e) => setForm({ ...form, [k]: e.target.value })} />
      <span className="muted ai-hint">{hint}</span>
    </label>
  );
  return (
    <div className="ai-product">
      <ProductHeader
        icon={<Coins size={20} />}
        name="AI Credits Management"
        actions={
          <Link className="link-muted ai-header-link" to="/assist/usage">
            Usage
          </Link>
        }
      />
      <div className="ai-page-pad ai-narrow">
        <section className="ai-card">
          <h2>This month</h2>
          <div className="ai-credits-big">
            <b>{compact(c.usedThisMonth)}</b> <span className="muted">credits used{c.monthlyLimit != null ? ` of ${compact(c.monthlyLimit)}` : ''}</span>
          </div>
          {c.monthlyLimit != null && (
            <div className={`ai-meter${pct >= 90 ? ' bad' : pct >= 75 ? ' warn' : ''}`}>
              <span style={{ width: `${pct}%` }} />
            </div>
          )}
          <p className="muted">
            You used {compact(c.myUsedToday)} credits today{c.userDailyLimit != null ? ` of your ${compact(c.userDailyLimit)}` : ''}. One credit is {compact(c.tokensPerCredit)} tokens the model reads
            or writes, in Assist Chat, investigations, security verdicts, agents and coding sessions.
          </p>
        </section>
        <section className="ai-card">
          <h2>Limits</h2>
          <p className="muted">Once reached, Assist stops starting new chats and runs until the limit renews. Admins only.</p>
          {field('monthlyLimit', 'Organization credits per month', 'Renews on the 1st (UTC).')}
          {field('userDailyLimit', 'Credits per member per day', 'Renews at midnight UTC. Automatic runs count only against the month.')}
          <h2>Token prices</h2>
          <p className="muted">What your provider charges, to estimate cost on the Usage page. Leave empty to show no cost.</p>
          {field('inputPricePerMtok', 'Input, USD per million tokens', 'e.g. 5 for Claude Opus 5', '0.01')}
          {field('outputPricePerMtok', 'Output, USD per million tokens', 'e.g. 25 for Claude Opus 5', '0.01')}
          <div className="ai-row">
            <button
              className="btn btn-primary btn-sm"
              disabled={save.isPending}
              onClick={() =>
                save.mutate({
                  monthlyLimit: num(form.monthlyLimit),
                  userDailyLimit: num(form.userDailyLimit),
                  inputPricePerMtok: num(form.inputPricePerMtok),
                  outputPricePerMtok: num(form.outputPricePerMtok),
                })
              }
            >
              {save.isPending ? 'Saving…' : 'Save'}
            </button>
            {save.isSuccess && <span className="ai-ok">Saved</span>}
            {save.error && <span className="ai-bad">{(save.error as Error).message}</span>}
          </div>
        </section>
      </div>
    </div>
  );
}

const APPS: Record<string, { label: string; icon: typeof Bot; to: string }> = {
  chat: { label: 'Assist Chat', icon: MessageSquare, to: '/assist' },
  investigation: { label: 'Assist Investigation', icon: Sparkles, to: '/assist/investigations' },
  security: { label: 'Assist Security Analyst', icon: ShieldCheck, to: '/assist/security-analyst' },
  agent: { label: 'Assist Agent Builder', icon: Bot, to: '/automation/agents' },
  code: { label: 'Assist Code', icon: Code2, to: '/assist/code' },
  testing: { label: 'AI Testing', icon: MousePointerClick, to: '/synthetics/ai-testing' },
};

function line(points: { at: number; v: number }[], name: string, color: string): EChartsOption {
  return {
    color: [color],
    grid: { left: 40, right: 12, top: 12, bottom: 24 },
    tooltip: { trigger: 'axis' },
    xAxis: { type: 'category', data: points.map((p) => new Date(p.at).toLocaleDateString('en-US', { month: 'short', day: 'numeric' })) },
    yAxis: { type: 'value', minInterval: 1 },
    series: [{ name, type: 'line', smooth: true, showSymbol: false, areaStyle: { opacity: 0.15 }, data: points.map((p) => p.v) }],
  };
}

function Usage() {
  const { range } = useTime();
  const r = useMemo(() => ({ from: Math.floor(range.from / 60_000) * 60_000, to: Math.ceil(range.to / 60_000) * 60_000 }), [range.from, range.to]);
  const q = useUsage(r);
  const colors = palette();
  const u = q.data;
  return (
    <div className="ai-product">
      <ProductHeader
        icon={<Activity size={20} />}
        name="Usage"
        actions={
          <>
            <Link className="link-muted ai-header-link" to="/assist/credits">
              AI Credits
            </Link>
            <TimePicker compact />
          </>
        }
      />
      <div className="ai-page-pad">
        {q.isError ? (
          <ErrorState error={q.error} onRetry={() => q.refetch()} />
        ) : !u ? (
          <Loading />
        ) : (
          <>
            {u.estCost == null && (
              <div className="ai-setup ai-row">
                <CircleAlert size={15} /> <b>No cost data.</b> Set your provider’s token prices in <Link to="/assist/credits">AI Credits Management</Link> to estimate cost.
              </div>
            )}
            <div className="ai-report-grid">
              <section className="ai-card ai-panel-wide">
                <h3>Est. cost</h3>
                <b className="ai-big">{usd(u.estCost)}</b>
                <span className="muted">
                  {compact(u.credits)} credits · {compact(u.inputTokens)} input / {compact(u.outputTokens)} output tokens
                </span>
                {u.daily.length ? (
                  <Chart
                    option={line(
                      u.daily.map((d) => ({ at: d.at, v: d.credits })),
                      'Credits',
                      colors[0],
                    )}
                    height={160}
                  />
                ) : (
                  <p className="muted ai-nodata">(No data)</p>
                )}
              </section>
              <section className="ai-card ai-panel-wide">
                <h3>Users</h3>
                <b className="ai-big">{u.users}</b>
                <span className="muted">Daily active users</span>
                {u.daily.length ? (
                  <Chart
                    option={line(
                      u.daily.map((d) => ({ at: d.at, v: d.users })),
                      'Users',
                      colors[1],
                    )}
                    height={160}
                  />
                ) : (
                  <p className="muted ai-nodata">(No data)</p>
                )}
              </section>
            </div>
            <table className="ai-table">
              <thead>
                <tr>
                  <th>APPLICATION</th>
                  <th>USERS</th>
                  <th>USES</th>
                  <th>CREDITS</th>
                  <th>TOTAL COST</th>
                </tr>
              </thead>
              <tbody>
                {u.byApp.map((a) => {
                  const app = APPS[a.app] ?? { label: a.app, icon: Bot, to: '/assist' };
                  return (
                    <tr key={a.app}>
                      <td>
                        <Link className="ai-row" to={app.to}>
                          <span className="ai-app-icon">
                            <app.icon size={14} />
                          </span>
                          {app.label}
                        </Link>
                      </td>
                      <td>{a.users}</td>
                      <td>{compact(a.uses)}</td>
                      <td>{compact(a.credits)}</td>
                      <td>{usd(a.estCost)}</td>
                    </tr>
                  );
                })}
              </tbody>
            </table>
            <h2 className="ai-list-head">Members</h2>
            <table className="ai-table">
              <thead>
                <tr>
                  <th>MEMBER</th>
                  <th>USES</th>
                  <th>CREDITS</th>
                </tr>
              </thead>
              <tbody>
                {u.byUser.map((m) => (
                  <tr key={m.userId}>
                    <td>{m.name}</td>
                    <td>{compact(m.uses)}</td>
                    <td>{compact(m.credits)}</td>
                  </tr>
                ))}
                {u.byUser.length === 0 && (
                  <tr>
                    <td colSpan={3} className="muted ai-empty-cell">
                      No use in this range.
                    </td>
                  </tr>
                )}
              </tbody>
            </table>
          </>
        )}
      </div>
    </div>
  );
}

export function UsagePage() {
  return (
    <TimeProvider initial="1mo">
      <Usage />
    </TimeProvider>
  );
}
