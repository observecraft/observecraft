import { useState } from 'react';
import { Search, X } from 'lucide-react';
import type { PortalReportDashboard } from '../../../api/portalCatalog/types';
import { Modal } from '../../../components/ui/Modal';
import { useDashboardChoices, useSetReportDashboards } from '../catalog/queries';

/** Pick the org's dashboards shown under Custom Reports. */
export function AddDashboardsModal({ current, onClose }: { current: PortalReportDashboard[]; onClose: () => void }) {
  const [query, setQuery] = useState('');
  const [chosen, setChosen] = useState<{ id: string; title: string }[]>(current.map((d) => ({ id: d.id, title: d.title })));
  const choices = useDashboardChoices(query);
  const save = useSetReportDashboards();
  const toggle = (id: string, title: string) => setChosen((c) => (c.some((x) => x.id === id) ? c.filter((x) => x.id !== id) : [...c, { id, title }]));
  return (
    <Modal onClose={onClose} label="Custom reports" width={640} height="auto">
      <div className="pr-modal">
        <header className="pr-modal-head">
          <h2>Custom Reports</h2>
          <button className="pr-icon-btn" aria-label="Close" onClick={onClose}>
            <X size={18} />
          </button>
        </header>
        <div className="pr-modal-body">
          <p className="pr-muted">Dashboards added here appear in the Reports navigation for everyone in the organization.</p>
          <label className="pr-search">
            <Search size={14} />
            <input value={query} onChange={(e) => setQuery(e.target.value)} placeholder="Search dashboards" aria-label="Search dashboards" />
          </label>
          <ul className="pr-choice-list">
            {(choices.data?.dashboards ?? []).map((d) => (
              <li key={d.id}>
                <label>
                  <input type="checkbox" checked={chosen.some((x) => x.id === d.id)} onChange={() => toggle(d.id, d.title)} />
                  <span>{d.title}</span>
                </label>
              </li>
            ))}
            {choices.data && !choices.data.dashboards.length && <li className="pr-muted">No dashboards match.</li>}
          </ul>
          {chosen.length > 0 && <p className="pr-muted">Selected: {chosen.map((c) => c.title).join(', ')}</p>}
          {save.isError && <div className="pr-error">{(save.error as Error).message}</div>}
          <div className="pr-modal-actions">
            <button className="btn" onClick={onClose}>
              Cancel
            </button>
            <button
              className="btn btn-primary"
              disabled={save.isPending}
              onClick={() =>
                save.mutate(
                  chosen.map((c) => c.id),
                  { onSuccess: onClose },
                )
              }
            >
              {save.isPending ? 'Saving…' : 'Save'}
            </button>
          </div>
        </div>
      </div>
    </Modal>
  );
}
