import { useState } from 'react';
import { Link } from 'react-router-dom';
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { dataSettings, listIndexes, updateDataSettings } from '../../api/logIndexes/client';
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
    mutationFn: (d: number) => updateDataSettings(d),
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
            </td>
            <td>{d.metricsRetentionDays} days</td>
            <td className="muted">Install-wide</td>
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
