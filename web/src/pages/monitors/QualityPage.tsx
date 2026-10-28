import { useState } from 'react';
import { Link, useNavigate } from 'react-router-dom';
import { Activity, Gauge, Pencil, Trash2 } from 'lucide-react';
import type { MonitorQuality } from '../../api/monitors/client';
import { ErrorState, Loading } from '../../components/ui/AsyncState';
import { Select } from '../../components/ui/Dropdown';
import { MuteMenu, toggle } from './common';
import { useBulk, useQuality } from './queries';

type Issue = MonitorQuality['issues'][number];

export function QualityPage() {
  const [creator, setCreator] = useState('*');
  const [team, setTeam] = useState('*');
  const [service, setService] = useState('*');
  const [env, setEnv] = useState('*');
  const pick = (v: string) => (v === '*' ? [] : [v]);
  const q = useQuality({
    creators: pick(creator),
    teams: pick(team),
    services: pick(service),
    envs: pick(env),
  });
  const d = q.data;
  const total = d ? d.healthy + d.toImprove : 0;
  const opts = (values: string[]) => [{ value: '*', label: '*' }, ...values.map((v) => ({ value: v, label: v }))];

  return (
    <div className="mon-quality">
      <p className="mon-q-kicker">Monitor Quality</p>
      <h1>Track and improve the health of your monitors</h1>
      <div className="mon-q-head">
        <h2>All monitors that need improvement</h2>
        <div className="mon-q-filters">
          <Select
            label="Creators"
            value={creator}
            onChange={setCreator}
            options={[
              { value: '*', label: '*' },
              ...(d?.creators ?? []).map((c) => ({
                value: c.id,
                label: c.name,
              })),
            ]}
          />
          <Select label="Teams" value={team} onChange={setTeam} options={opts(d?.teams ?? [])} />
          <Select label="Services" value={service} onChange={setService} options={opts(d?.services ?? [])} />
          <Select label="Envs" value={env} onChange={setEnv} options={opts(d?.envs ?? [])} />
        </div>
      </div>
      {q.isPending ? (
        <Loading label="Checking monitors…" />
      ) : q.isError ? (
        <ErrorState error={q.error} onRetry={() => q.refetch()} />
      ) : (
        <>
          <div className="mon-q-bar" role="img" aria-label={`${d!.healthy} healthy monitors, ${d!.toImprove} to improve`}>
            <div style={{ width: `${total ? (d!.healthy / total) * 100 : 100}%` }} />
          </div>
          <div className="mon-q-legend">
            <span>
              <b className="link">{d!.healthy.toLocaleString()}</b> Healthy monitors
            </span>
            <span>
              <b>{d!.toImprove.toLocaleString()}</b> Monitors to improve
            </span>
          </div>
          {d!.issues.length === 0 && <p className="mon-empty">Every monitor in scope looks healthy.</p>}
          {d!.issues.map((i) => (
            <IssueCard key={i.kind} issue={i} />
          ))}
        </>
      )}
    </div>
  );
}

function IssueCard({ issue }: { issue: Issue }) {
  const navigate = useNavigate();
  const bulk = useBulk();
  const [selected, setSelected] = useState<number[]>([]);
  const ids = issue.monitors.map((m) => m.id);
  const all = ids.length > 0 && ids.every((id) => selected.includes(id));
  const remove = (list: number[]) =>
    window.confirm(`Delete ${list.length} monitor${list.length === 1 ? '' : 's'}? This can't be undone.`) && bulk.mutate({ ids: list, action: 'delete' }, { onSuccess: () => setSelected([]) });
  return (
    <section className="mon-q-card">
      <header>
        <div>
          <h3>{issue.title}</h3>
          <p className="muted">{issue.description}</p>
        </div>
        <span className="mon-q-fix">{issue.fix}</span>
      </header>
      <table className="mon-table mon-q-table">
        <thead>
          <tr>
            <th className="col-check">
              <input type="checkbox" className="checkbox" checked={all} onChange={() => setSelected(all ? [] : ids)} aria-label="Select all" />
            </th>
            <th>MONITOR</th>
            <th className="mon-q-actions-col">ACTIONS</th>
          </tr>
          <tr className="mon-bulk">
            <th colSpan={3}>
              <MuteMenu disabled={!selected.length} onMute={(m) => bulk.mutate({ ids: selected, action: 'mute', muteMinutes: m }, { onSuccess: () => setSelected([]) })} />
              <button className="btn" disabled={!selected.length} onClick={() => remove(selected)}>
                <Trash2 size={14} /> Delete
              </button>
            </th>
          </tr>
        </thead>
        <tbody>
          {issue.monitors.map((m) => (
            <tr key={m.id}>
              <td className="col-check">
                <input type="checkbox" className="checkbox" checked={selected.includes(m.id)} onChange={() => setSelected(toggle(selected, m.id))} aria-label={`Select ${m.name}`} />
              </td>
              <td>
                <div className="mon-q-mon">
                  <span className={`mon-q-badge q-${issue.kind}`}>{m.detail.toUpperCase()}</span>
                  <span className="mon-q-type">
                    {m.type === 'apm' ? <Activity size={15} /> : <Gauge size={15} />}{' '}
                    {m.type === 'apm' ? 'APM Monitor' : m.type === 'log' ? 'Logs Monitor' : m.type === 'synthetics' ? 'Synthetics Monitor' : 'Metric Monitor'}
                  </span>
                  <Link to={`/monitors/${m.id}`}>{m.name}</Link>
                </div>
              </td>
              <td>
                <div className="mon-q-actions">
                  <button className="btn btn-primary" onClick={() => navigate(`/monitors/${m.id}/edit`)}>
                    <Pencil size={14} /> Edit
                  </button>
                  <button className="btn mon-btn-danger" onClick={() => remove([m.id])}>
                    <Trash2 size={14} /> Delete
                  </button>
                </div>
              </td>
            </tr>
          ))}
        </tbody>
      </table>
    </section>
  );
}
