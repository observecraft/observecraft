import { useState } from 'react';
import { Search } from 'lucide-react';
import { emptyTrigger, STEP_FIELDS, STEP_LABEL } from '../../../api/workflows/logic';
import type { WorkflowStepKind } from '../../../api/workflows/types';
import { CATALOG, ID_BASE, starterStep } from './catalog';
import { LogoChip, StepTile } from './common';
import { useCreateDraft } from './useStartFromBlueprint';
import './workflows.css';

/** Automation → Action Catalog: what workflow steps can do. */
export function ActionCatalogPage() {
  const create = useCreateDraft();
  const [q, setQ] = useState('');
  const t = q.trim().toLowerCase();
  const use = (kind: WorkflowStepKind) => {
    const id = ID_BASE[kind];
    create(
      {
        name: `New workflow: ${STEP_LABEL[kind]}`,
        handle: '',
        description: '',
        tags: [],
        ownerTeam: '',
        service: '',
        blueprint: '',
        spec: { inputs: [], triggers: [emptyTrigger('manual')], steps: [starterStep(id, kind)] },
      },
      id,
    );
  };
  return (
    <div className="wfc-page">
      <div className="wfb-head">
        <div>
          <h2>Action Catalog</h2>
          <p className="wf-muted">Every step a workflow can run. HTTP requests reach any tool with an API; the others act inside ObserveCraft.</p>
        </div>
        <div className="wfl-search wfb-search">
          <Search size={15} />
          <input placeholder="Search actions" value={q} onChange={(e) => setQ(e.target.value)} />
        </div>
      </div>
      {create.error && <div className="wf-error">{create.error.message}</div>}
      {CATALOG.map((group) => {
        const entries = group.entries.filter((e) => !t || `${STEP_LABEL[e.kind]} ${e.description} ${group.category}`.toLowerCase().includes(t));
        if (!entries.length) return null;
        return (
          <section key={group.category} className="wfc-group">
            <h3>{group.category}</h3>
            <div className="wfc-grid">
              {entries.map((e) => (
                <article key={e.kind} className="wfc-card">
                  <div className="wfc-card-head">
                    <StepTile kind={e.kind} size={34} />
                    <div>
                      <div className="wfc-title">{STEP_LABEL[e.kind]}</div>
                      <div className="wf-muted wfc-fields">
                        {STEP_FIELDS[e.kind].map(([f, req]) => `${f}${req ? '' : '?'}`).join(' · ') || (e.kind === 'http' ? 'method · url · headers · body' : 'expression · steps')}
                      </div>
                    </div>
                  </div>
                  <p>{e.description}</p>
                  <div className="wfb-actions">
                    <span className="wfl-mini-logos">
                      {(e.logos ?? []).map((l) => (
                        <LogoChip key={l} logo={l} size={22} />
                      ))}
                    </span>
                    <button className="btn btn-sm" disabled={create.pending} onClick={() => use(e.kind)}>
                      Use in a new workflow
                    </button>
                  </div>
                </article>
              ))}
            </div>
          </section>
        );
      })}
    </div>
  );
}
