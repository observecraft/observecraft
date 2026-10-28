import { useDeferredValue, useState } from 'react';
import { LayoutDashboard, Plus } from 'lucide-react';
import type { DashboardSummary } from '../../api/dashboards/types';
import { Modal } from '../../components/ui/Modal';
import { timeAgo } from '../../lib/format';
import { useDashboardSearch } from './queries';
import '../notebooks/notebooks.css';

/** Picks a dashboard (custom ones only when `editableOnly`); optionally offers "New dashboard". */
export function DashboardPickerModal({
  title,
  editableOnly,
  onPick,
  onNew,
  busy,
  error,
  onClose,
}: {
  title: string;
  editableOnly?: boolean;
  onPick: (d: DashboardSummary) => void;
  onNew?: () => void;
  busy?: boolean;
  error?: string | null;
  onClose: () => void;
}) {
  const [query, setQuery] = useState('');
  const search = useDashboardSearch({
    query: useDeferredValue(query),
    preset: editableOnly ? 'custom' : 'all',
    listId: null,
    team: null,
    sort: { field: 'modified', dir: 'desc' },
    offset: 0,
    limit: 20,
  });
  const rows = search.data?.dashboards ?? [];
  return (
    <Modal onClose={onClose} label={title} width={520} height="auto">
      <div className="small-modal">
        <h3>{title}</h3>
        {onNew && (
          <button className="btn btn-primary" disabled={busy} onClick={onNew}>
            <Plus size={14} /> New dashboard
          </button>
        )}
        <input autoFocus placeholder={onNew ? '…or find a dashboard' : 'Find a dashboard'} value={query} onChange={(e) => setQuery(e.target.value)} />
        <div className="nb-pick-list">
          {rows.map((d) => (
            <button key={d.id} disabled={busy} onClick={() => onPick(d)}>
              <span>
                <LayoutDashboard size={14} className="muted" /> {d.title}
              </span>
              <span className="muted-text">{timeAgo(d.modifiedAt)}</span>
            </button>
          ))}
          {search.data && rows.length === 0 && <p className="muted-text nb-empty">No dashboard matches.</p>}
        </div>
        {error && <p className="form-error">{error}</p>}
      </div>
    </Modal>
  );
}
