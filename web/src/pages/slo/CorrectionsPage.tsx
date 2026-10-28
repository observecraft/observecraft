import { useState } from 'react';
import { Link } from 'react-router-dom';
import { Pencil, Plus, Search, Trash2 } from 'lucide-react';
import type { SloCorrection } from '../../api/slos/client';
import { ErrorState, Loading } from '../../components/ui/AsyncState';
import { CATEGORY_LABEL, fmtDateTime, fmtMinutes } from './common';
import { CorrectionModal } from './CorrectionModal';
import { useCorrections, useDeleteCorrection, useSlos } from './queries';

const ALL_SLOS = { query: '', types: [], states: [], burnRates: [], creators: [], teams: [], services: [], envs: [], tags: [], window: null, sort: 'name', offset: 0, limit: 200 };

/** Corrections: time ranges excluded from SLO calculations, per SLO or for every SLO. */
export function CorrectionsPage() {
  const q = useCorrections();
  const slos = useSlos(ALL_SLOS);
  const remove = useDeleteCorrection();
  const [filter, setFilter] = useState('');
  const [editing, setEditing] = useState<SloCorrection | 'new' | null>(null);
  const words = filter.toLowerCase().split(/\s+/).filter(Boolean);
  const rows = (q.data ?? []).filter((c) => {
    const hay = `${c.sloName ?? 'every slo'} ${c.description} ${CATEGORY_LABEL[c.category]}`.toLowerCase();
    return words.every((w) => hay.includes(w));
  });
  const now = Date.now();

  return (
    <div className="mon-page">
      <div className="mon-filter">
        <Search size={16} />
        <input placeholder="Filter corrections (SLO, reason or description)" value={filter} onChange={(e) => setFilter(e.target.value)} />
      </div>
      <div className="slo-corr-page">
        <div className="mon-toolbar">
          <span>
            <b>{rows.length}</b> correction{rows.length === 1 ? '' : 's'}
          </span>
          <span className="mon-toolbar-right">
            <button className="btn btn-primary" onClick={() => setEditing('new')}>
              <Plus size={15} /> New Correction
            </button>
          </span>
        </div>
        {remove.isError && <p className="form-error">{(remove.error as Error).message}</p>}
        {q.isPending ? (
          <Loading label="Loading corrections…" />
        ) : q.isError ? (
          <ErrorState error={q.error} onRetry={() => q.refetch()} />
        ) : (
          <div className="mon-table-wrap">
            <table className="mon-table">
              <thead>
                <tr>
                  <th>SLO</th>
                  <th>REASON</th>
                  <th>START</th>
                  <th>END</th>
                  <th>DURATION</th>
                  <th>DESCRIPTION</th>
                  <th>CREATOR</th>
                  <th />
                </tr>
              </thead>
              <tbody>
                {rows.map((c) => (
                  <tr key={c.id} className={c.end < now ? 'slo-past' : ''}>
                    <td>{c.sloId ? <Link to={`/slo?slo=${encodeURIComponent(c.sloId)}`}>{c.sloName ?? c.sloId}</Link> : <span className="mon-tag">Every SLO</span>}</td>
                    <td>{CATEGORY_LABEL[c.category]}</td>
                    <td>{fmtDateTime(c.start)}</td>
                    <td>{fmtDateTime(c.end)}</td>
                    <td>{fmtMinutes((c.end - c.start) / 60_000)}</td>
                    <td>{c.description}</td>
                    <td>{c.creator?.name ?? '—'}</td>
                    <td className="slo-row-actions">
                      <button className="icon-plain" title="Edit" onClick={() => setEditing(c)}>
                        <Pencil size={14} />
                      </button>
                      <button className="icon-plain" title="Delete" onClick={() => window.confirm('Delete this correction? The SLOs it applied to are recomputed without it.') && remove.mutate(c.id)}>
                        <Trash2 size={14} />
                      </button>
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
            {rows.length === 0 && <p className="mon-empty">{q.data.length === 0 ? 'No corrections yet. Add one to leave planned maintenance out of your SLOs.' : 'No corrections match.'}</p>}
          </div>
        )}
      </div>
      {editing && <CorrectionModal onClose={() => setEditing(null)} correction={editing === 'new' ? undefined : editing} slos={slos.data?.slos ?? []} />}
    </div>
  );
}
