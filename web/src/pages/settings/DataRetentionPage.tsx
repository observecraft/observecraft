import { useState } from 'react';
import { Link } from 'react-router-dom';
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { dataSettings, listIndexes, updateDataSettings, type MetricRetention, type OrgDataSettings } from '../../api/logIndexes/client';
import { ErrorState, Loading } from '../../components/ui/AsyncState';
import { useProfile } from './queries';

const SPAN_DAYS = [3, 7, 15, 30, 45, 60, 90];

/** Organization Settings → Data Retention: how long each kind of telemetry is kept for this org. */
export function DataRetentionPage() {
  const qc = useQueryClient();
  const me = useProfile();
  const isAdmin = me.data?.roles.some((r) => r.id === `${me.data?.org.id}-admin`) ?? false;
  const s = useQuery({ queryKey: ['data-settings'], queryFn: dataSettings });
  const ix = useQuery({ queryKey: ['log-indexes'], queryFn: listIndexes });
  const [span, setSpan] = useState<number | null>(null);
  const save = useMutation({
    mutationFn: (d: number) => updateDataSettings({ spanRetentionDays: d }),
    onSuccess: () => {
      setSpan(null);
      void qc.invalidateQueries({ queryKey: ['data-settings'] });
    },
  });
  if (s.isPending) return <Loading />;
  if (s.isError) return <ErrorState error={s.error} onRetry={() => s.refetch()} />;
  const d = s.data;
  const value = span ?? d.spanRetentionDays;
  const options = SPAN_DAYS.filter((x) => x <= d.maxSpanRetentionDays);
  if (!options.includes(d.spanRetentionDays)) options.push(d.spanRetentionDays);
  return (
    <div className="settings-pad">
      <h1 className="settings-title">Data Retention</h1>
      <p className="muted">How long this organization's telemetry stays searchable. Limits are set by the administrators of this ObserveCraft install.</p>
      <table className="settings-table dr-table">
        <thead>
          <tr>
            <th>DATA</th>
            <th>RETENTION</th>
            <th>CONFIGURE</th>
          </tr>
        </thead>
        <tbody>
          <tr>
            <td>
              <b>Logs</b>
              <div className="muted small">Per index: each index keeps its logs for its own retention.</div>
            </td>
            <td>
              {ix.data?.indexes.length ? ix.data.indexes.map((i) => <div key={i.id}>{`${i.name}: ${i.retentionDays} days`}</div>) : <span className="muted">No indexes</span>}
              <div className="muted small">Up to {d.maxLogRetentionDays} days</div>
            </td>
            <td>
              <Link to="/logs/indexes">Log Indexes</Link>
            </td>
          </tr>
          <tr>
            <td>
              <b>Spans (APM traces)</b>
              <div className="muted small">Applies to spans received from now on.</div>
            </td>
            <td>
              {isAdmin ? (
                <span className="dr-edit">
                  <select value={value} onChange={(e) => setSpan(Number(e.target.value))}>
                    {options
                      .sort((a, b) => a - b)
                      .map((x) => (
                        <option key={x} value={x}>
                          {x} days
                        </option>
                      ))}
                  </select>
                  {span !== null && span !== d.spanRetentionDays && (
                    <button className="btn btn-primary btn-sm" onClick={() => save.mutate(span)} disabled={save.isPending}>
                      Save
                    </button>
                  )}
                </span>
              ) : (
                `${d.spanRetentionDays} days`
              )}
              <div className="muted small">Up to {d.maxSpanRetentionDays} days</div>
            </td>
            <td className="muted">Here</td>
          </tr>
          <tr>
            <td>
              <b>Log archives</b>
              <div className="muted small">Your own bucket: kept as long as your bucket's lifecycle rules say.</div>
            </td>
            <td className="muted">Unlimited</td>
            <td>
              <Link to="/logs/archives">Archiving &amp; Forwarding</Link>
            </td>
          </tr>
          <tr>
            <td>
              <b>Metrics</b>
              <div className="muted small">Long ranges read the rollups. Applies to metrics received from now on.</div>
            </td>
            <td>
              <MetricRetentionEdit settings={d} editable={isAdmin} />
            </td>
            <td className="muted">Here</td>
          </tr>
          <tr>
            <td>
              <b>CI pipelines and tests</b>
            </td>
            <td>{d.ciRetentionDays} days</td>
            <td className="muted">Install-wide</td>
          </tr>
          <tr>
            <td>
              <b>Rehydrated logs</b>
            </td>
            <td className="muted">Chosen per rehydration</td>
            <td>
              <Link to="/logs/rehydrations">Rehydrations</Link>
            </td>
          </tr>
        </tbody>
      </table>
      {save.error && <p className="form-error">{(save.error as Error).message}</p>}
      <p className="muted small">Every log is also kept unindexed for {Math.round(d.rawLogsHours / 24)} days so archives receive it, even when no index keeps it.</p>
    </div>
  );
}

const METRIC_LEVELS: { key: keyof MetricRetention; label: string }[] = [
  { key: 'raw', label: 'Full resolution' },
  { key: 'rollup5m', label: '5-minute rollups' },
  { key: 'rollup1h', label: '1-hour rollups' },
];

/** The org's metric retention per resolution, editable by org admins within the install's limits. */
function MetricRetentionEdit({ settings: d, editable }: { settings: OrgDataSettings; editable: boolean }) {
  const qc = useQueryClient();
  const [draft, setDraft] = useState<MetricRetention | null>(null);
  const save = useMutation({
    mutationFn: (metrics: MetricRetention) => updateDataSettings({ metrics }),
    onSuccess: () => {
      setDraft(null);
      void qc.invalidateQueries({ queryKey: ['data-settings'] });
    },
  });
  const value = draft ?? d.metrics;
  const changed = draft !== null && METRIC_LEVELS.some(({ key }) => draft[key] !== d.metrics[key]);
  return (
    <>
      {METRIC_LEVELS.map(({ key, label }) => {
        const options = d.metricRetentionTiers.filter((x) => x <= d.maxMetrics[key]);
        if (!options.includes(d.metrics[key])) options.push(d.metrics[key]);
        return (
          <div key={key} className="dr-edit">
            <span className="muted">{label}:</span>{' '}
            {editable ? (
              <select value={value[key]} onChange={(e) => setDraft({ ...value, [key]: Number(e.target.value) })}>
                {options
                  .sort((a, b) => a - b)
                  .map((x) => (
                    <option key={x} value={x}>
                      {x} days
                    </option>
                  ))}
              </select>
            ) : (
              `${d.metrics[key]} days`
            )}
          </div>
        );
      })}
      {changed && (
        <button className="btn btn-primary btn-sm" onClick={() => save.mutate(value)} disabled={save.isPending}>
          Save
        </button>
      )}
      <div className="muted small">
        Up to {d.maxMetrics.raw} / {d.maxMetrics.rollup5m} / {d.maxMetrics.rollup1h} days
      </div>
      {save.error && <p className="form-error">{(save.error as Error).message}</p>}
    </>
  );
}
