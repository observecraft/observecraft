import { useState } from 'react';
import { Search, X } from 'lucide-react';
import { STEP_LABEL } from '../../../api/workflows/logic';
import type { WorkflowStepKind } from '../../../api/workflows/types';
import { CATALOG } from './catalog';
import { StepTile } from './common';

/** Left drawer of the editor: pick the kind of step to add at the chosen "+". */
export function CatalogDrawer({ onPick, onClose }: { onPick: (kind: WorkflowStepKind) => void; onClose: () => void }) {
  const [q, setQ] = useState('');
  const t = q.trim().toLowerCase();
  return (
    <aside className="wfe-drawer" onClick={(e) => e.stopPropagation()}>
      <div className="wfe-panel-head">
        <div className="wfe-panel-title">
          <div>Action catalog</div>
          <div className="wf-muted">Add a step here</div>
        </div>
        <button className="wf-icon-btn" aria-label="Close the catalog" onClick={onClose}>
          <X size={16} />
        </button>
      </div>
      <div className="wfl-search wfe-drawer-search">
        <Search size={14} />
        <input autoFocus placeholder="Search actions" value={q} onChange={(e) => setQ(e.target.value)} />
      </div>
      <div className="wfe-drawer-body">
        {CATALOG.map((g) => {
          const entries = g.entries.filter((e) => !t || `${STEP_LABEL[e.kind]} ${e.description} ${g.category}`.toLowerCase().includes(t));
          if (!entries.length) return null;
          return (
            <div key={g.category} className="wfe-drawer-group">
              <div className="wfv-group">{g.category}</div>
              {entries.map((e) => (
                <button key={e.kind} className="wfe-drawer-item" onClick={() => onPick(e.kind)}>
                  <StepTile kind={e.kind} size={28} />
                  <span>
                    <span className="wfe-drawer-title">{STEP_LABEL[e.kind]}</span>
                    <span className="wf-muted wfe-drawer-desc">{e.description}</span>
                  </span>
                </button>
              ))}
            </div>
          );
        })}
      </div>
    </aside>
  );
}
