import { useEffect, useState } from 'react';
import { Link } from 'react-router-dom';
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { Check } from 'lucide-react';
import { listIndexes } from '../../api/logIndexes/client';
import { getFlexSettings, updateFlexSettings } from '../../api/logsConfig/client';
import { ErrorState, Loading } from '../../components/ui/AsyncState';
import { useProfile } from '../settings/queries';
import { ConfigTitle } from './config/configUi';

const SIZES = ['xs', 'xs+', 's', 'm', 'l'];

/** Log Configuration → Flex Logs Controls: the flex tier (longer, cheaper-to-keep retention) and its compute. */
export function FlexLogsPage() {
  const qc = useQueryClient();
  const me = useProfile();
  const isAdmin = me.data?.roles.some((r) => r.id === `${me.data?.org.id}-admin`) ?? false;
  const q = useQuery({ queryKey: ['log-flex-settings'], queryFn: getFlexSettings });
  const indexes = useQuery({ queryKey: ['log-indexes'], queryFn: listIndexes });
  const [enabled, setEnabled] = useState(false);
  const [compute, setCompute] = useState('starter');
  const [size, setSize] = useState<string | null>(null);
  useEffect(() => {
    if (q.data) {
      setEnabled(q.data.enabled);
      setCompute(q.data.compute);
      setSize(q.data.size ?? null);
    }
  }, [q.data]);
  const save = useMutation({
    mutationFn: () => updateFlexSettings({ enabled, compute, size: compute === 'scalable' ? (size ?? 'xs') : null }),
    onSuccess: () => (qc.invalidateQueries({ queryKey: ['log-flex-settings'] }), qc.invalidateQueries({ queryKey: ['log-indexes'] })),
  });

  if (q.isPending) return <Loading />;
  if (q.isError) return <ErrorState error={q.error} onRetry={() => q.refetch()} />;
  const flexIndexes = (indexes.data?.indexes ?? []).filter((i) => i.flexRetentionDays != null);
  const changed = enabled !== q.data.enabled || compute !== q.data.compute || (compute === 'scalable' && (size ?? 'xs') !== (q.data.size ?? null));
  const option = (id: string, title: string, sub: string, points: string[]) => (
    <button type="button" className={`lc-option${compute === id ? ' selected' : ''}`} disabled={!isAdmin} onClick={() => setCompute(id)}>
      <div className="lc-option-head">
        <b>
          {compute === id && <Check size={14} />} {title}
        </b>
        <span>{sub}</span>
      </div>
      {id === 'scalable' && compute === 'scalable' && (
        <div className="lc-sizes" onClick={(e) => e.stopPropagation()}>
          {SIZES.map((s) => (
            <button type="button" key={s} className={(size ?? 'xs') === s ? 'on' : ''} disabled={!isAdmin} onClick={() => setSize(s)}>
              {s.toUpperCase()}
            </button>
          ))}
        </div>
      )}
      <ul>
        {points.map((p) => (
          <li key={p}>{p}</li>
        ))}
      </ul>
    </button>
  );

  return (
    <div className="lc-page">
      <ConfigTitle title="Flex Logs Controls" />
      <p className="lc-subtitle">Keep logs longer than their index's standard retention, for investigations and audits that reach further back.</p>
      <section className="lc-banner">
        <label className="ar-check">
          <button
            role="switch"
            aria-checked={enabled}
            aria-label={enabled ? 'Turn Flex Logs off' : 'Turn Flex Logs on'}
            className={`switch${enabled ? ' on' : ''}`}
            disabled={!isAdmin}
            onClick={() => setEnabled((e) => !e)}
          />{' '}
          <b>Flex Logs {enabled ? 'on' : 'off'}</b>
        </label>
        <p className="muted">
          With Flex Logs on, an index with a flex tier keeps its logs for the flex retention (its total time: standard + flex) instead of its standard retention. Set the flex tier per index in{' '}
          <Link to="/logs/indexes">Indexes</Link> or with Terraform (<code>flex_retention_days</code>). It applies to logs indexed from then on; logs stay searchable the same way.
        </p>
      </section>
      <h3 className="lc-h3">Flex Compute Options</h3>
      <div className="lc-options">
        {option('starter', 'Starter', 'One compute size for smaller organizations', ['Suited to up to about 10 billion stored logs', 'Flex retention of 1 to 15 months', 'A single compute size'])}
        {option('scalable', 'Scalable Compute', 'A compute size that grows with larger organizations', [
          'Suited to more than 10 billion stored logs',
          'Flex retention of 1 to 15 months',
          'Compute sizes XS, XS+, S, M and L',
        ])}
      </div>
      <p className="muted">The compute choice is recorded for the organization; flex-tier logs are searched by the same engine as the standard tier.</p>
      {save.error && <p className="form-error">{(save.error as Error).message}</p>}
      {isAdmin && (
        <div>
          <button className="btn btn-primary" disabled={!changed || save.isPending} onClick={() => save.mutate()}>
            {save.isPending ? 'Saving…' : 'Save'}
          </button>
        </div>
      )}
      <h3 className="lc-h3">Indexes with a flex tier</h3>
      {flexIndexes.length === 0 ? (
        <p className="muted">No index has a flex tier yet.</p>
      ) : (
        <section className="lc-section">
          {flexIndexes.map((ix) => (
            <div key={ix.id} className="lc-row" style={{ gridTemplateColumns: '1fr 160px 200px' }}>
              <span className="mono">{ix.name}</span>
              <span>{ix.retentionDays} days standard</span>
              <span>
                {ix.flexRetentionDays} days in total {!q.data.enabled && <span className="lc-pill muted">not applied</span>}
              </span>
            </div>
          ))}
        </section>
      )}
    </div>
  );
}
