// Assist Security Analyst: verdicts on Cloud SIEM signals (benign, suspicious, malicious) with evidence,
// by hand or automatically for the severities the org chooses.
import { useEffect, useState } from 'react';
import { Link, useNavigate, useParams } from 'react-router-dom';
import { Play, ShieldCheck, Sparkles } from 'lucide-react';
import { TimePicker } from '../../components/shell/PageHeader';
import { TimeProvider, useTime } from '../../components/shell/TimeContext';
import { ErrorState, Loading } from '../../components/ui/AsyncState';
import { timeAgo } from '../../lib/format';
import { ProductHeader, RunView, StatusBadge, VerdictBadge } from './RunView';
import { useAnalyzedSignals, useRunSettings, useSaveRunSettings, useStartRun } from './runsQueries';
import './assist.css';

const SEVERITIES = ['critical', 'high', 'medium', 'low', 'info'];

function AutoSettings() {
  const q = useRunSettings();
  const save = useSaveRunSettings();
  const [chosen, setChosen] = useState<string[]>([]);
  useEffect(() => {
    if (q.data) setChosen(q.data.securityAutoSeverities);
  }, [q.data]);
  if (!q.data) return null;
  const toggle = (s: string) => setChosen((c) => (c.includes(s) ? c.filter((x) => x !== s) : [...c, s]));
  return (
    <div className="ai-card ai-row ai-wrap">
      <b>Judge new signals automatically:</b>
      {SEVERITIES.map((s) => (
        <label key={s} className={`ai-chip sev-${s}${chosen.includes(s) ? ' on' : ''}`}>
          <input type="checkbox" className="checkbox" checked={chosen.includes(s)} onChange={() => toggle(s)} /> {s}
        </label>
      ))}
      <button className="btn btn-sm" disabled={save.isPending || chosen.join() === q.data.securityAutoSeverities.join()} onClick={() => save.mutate({ ...q.data!, securityAutoSeverities: chosen })}>
        Save
      </button>
      {save.error && <span className="ai-bad">{(save.error as Error).message}</span>}
      <span className="muted">Runs act as the admin who saves this, and count against Rate Limits.</span>
    </div>
  );
}

function Signals() {
  const navigate = useNavigate();
  const { range } = useTime();
  const q = useAnalyzedSignals({ from: Math.floor(range.from / 60_000) * 60_000, to: Math.ceil(range.to / 60_000) * 60_000 });
  const start = useStartRun();
  const [verdict, setVerdict] = useState('*');
  const rows = (q.data ?? []).filter((s) => verdict === '*' || (verdict === 'none' ? !s.run : s.run?.verdict === verdict));
  const counts = (v: string) => (q.data ?? []).filter((s) => s.run?.verdict === v).length;
  return (
    <div className="ai-page-pad">
      <section className="ai-hero">
        <div className="ai-hero-icon">
          <Sparkles size={22} />
        </div>
        <div>
          <h1>Autonomously investigate signals</h1>
          <p>Assist reads each signal, its rule and the logs behind it, compares them with your organization’s usual activity, and gives a verdict with the evidence, in minutes.</p>
        </div>
      </section>
      <AutoSettings />
      <div className="ai-row ai-spread">
        <div className="ai-row ai-segment">
          {[
            ['*', `All ${q.data?.length ?? ''}`],
            ['malicious', `Malicious ${counts('malicious')}`],
            ['suspicious', `Suspicious ${counts('suspicious')}`],
            ['benign', `Benign ${counts('benign')}`],
            ['none', 'Not investigated'],
          ].map(([v, label]) => (
            <button key={v} className={verdict === v ? 'active' : ''} onClick={() => setVerdict(v)}>
              {label}
            </button>
          ))}
        </div>
        <TimePicker compact />
      </div>
      {q.isPending ? (
        <Loading />
      ) : q.isError ? (
        <ErrorState error={q.error} onRetry={() => q.refetch()} />
      ) : (
        <table className="ai-table">
          <thead>
            <tr>
              <th>SEVERITY</th>
              <th>SIGNAL</th>
              <th>RULE</th>
              <th>CREATED</th>
              <th>ASSIST</th>
            </tr>
          </thead>
          <tbody>
            {rows.map((s) => (
              <tr key={s.id}>
                <td>
                  <span className={`ai-sev sev-${s.severity}`}>{s.severity}</span>
                </td>
                <td>
                  <Link to={`/security/siem/signals?signal=${encodeURIComponent(s.id)}`}>{s.title}</Link>
                </td>
                <td className="muted">{s.ruleName}</td>
                <td className="muted">{timeAgo(s.createdAt)}</td>
                <td>
                  {s.run ? (
                    <Link className="ai-row" to={`/assist/security-analyst/${s.run.id}`}>
                      {s.run.verdict ? <VerdictBadge verdict={s.run.verdict} /> : <StatusBadge status={s.run.status} />}
                    </Link>
                  ) : (
                    <button
                      className="btn btn-sm"
                      disabled={start.isPending}
                      onClick={() => start.mutate({ kind: 'security', signalId: s.id, monitorId: null, question: null }, { onSuccess: (r) => navigate(`/assist/security-analyst/${r.id}`) })}
                    >
                      <Play size={12} /> Investigate
                    </button>
                  )}
                </td>
              </tr>
            ))}
            {rows.length === 0 && (
              <tr>
                <td colSpan={5} className="muted ai-empty-cell">
                  No signals in this range. Cloud SIEM raises them from your detection rules.
                </td>
              </tr>
            )}
          </tbody>
        </table>
      )}
      {start.error && <p className="ai-bad">{(start.error as Error).message}</p>}
    </div>
  );
}

export function SecurityAnalystPage() {
  return (
    <div className="ai-product">
      <ProductHeader icon={<ShieldCheck size={20} />} name="Assist Security Analyst" />
      <TimeProvider initial="1w">
        <Signals />
      </TimeProvider>
    </div>
  );
}

export function SecurityRunPage() {
  const { id } = useParams();
  return (
    <div className="ai-product">
      <ProductHeader icon={<ShieldCheck size={20} />} name="Assist Security Analyst" />
      <RunView id={id!} back="/assist/security-analyst" backLabel="Signals" />
    </div>
  );
}
