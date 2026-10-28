import { useState } from 'react';
import { Search } from 'lucide-react';
import { allSteps, STEP_LABEL, TRIGGER_LABEL } from '../../../api/workflows/logic';
import { BLUEPRINTS, type BlueprintCategory } from './blueprints';
import { LogoChip, TRIGGER_ICON } from './common';
import { useStartFromBlueprint } from './useStartFromBlueprint';
import './workflows.css';

const CATEGORIES: (BlueprintCategory | 'All')[] = ['All', 'Incident response', 'Remediation', 'Reporting', 'Developer workflows'];

/** Workflow Automation → Blueprints. */
export function WorkflowsBlueprints() {
  const start = useStartFromBlueprint();
  const [q, setQ] = useState('');
  const [cat, setCat] = useState<BlueprintCategory | 'All'>('All');
  const t = q.trim().toLowerCase();
  const list = BLUEPRINTS.filter((b) => (cat === 'All' || b.category === cat) && (!t || `${b.title} ${b.description} ${b.keywords.join(' ')}`.toLowerCase().includes(t)));
  return (
    <div className="wfb-page">
      <div className="wfb-head">
        <div>
          <h2>Blueprints</h2>
          <p className="wf-muted">Ready-made workflows. Using one creates a draft you can change before publishing.</p>
        </div>
        <div className="wfl-search wfb-search">
          <Search size={15} />
          <input placeholder="Search blueprints" value={q} onChange={(e) => setQ(e.target.value)} />
        </div>
      </div>
      <div className="wfb-cats">
        {CATEGORIES.map((c) => (
          <button key={c} className={`wfb-cat${c === cat ? ' active' : ''}`} onClick={() => setCat(c)}>
            {c}
          </button>
        ))}
      </div>
      {start.error && <div className="wf-error">{start.error.message}</div>}
      <div className="wfb-grid">
        {list.map((b) => {
          const steps = allSteps(b.build().spec.steps);
          return (
            <article key={b.key} className="wfb-card">
              <div className="wfl-mini-logos">
                {b.logos.map((l) => (
                  <LogoChip key={l} logo={l} size={30} />
                ))}
              </div>
              <h3>{b.title}</h3>
              <p>{b.description}</p>
              <div className="wfb-meta">
                <span className="wfl-trigger">
                  {TRIGGER_ICON[b.trigger]} {TRIGGER_LABEL[b.trigger]}
                </span>
                <span className="wf-muted">
                  {steps.length} step{steps.length === 1 ? '' : 's'}
                </span>
                <span className="wf-muted wfb-kinds">{[...new Set(steps.map((s) => STEP_LABEL[s.kind]))].slice(0, 3).join(' · ')}</span>
              </div>
              <div className="wfb-actions">
                <span className="wfb-category">{b.category}</span>
                <button className="btn btn-sm btn-primary" disabled={start.pending} onClick={() => start(b)}>
                  Use blueprint
                </button>
              </div>
            </article>
          );
        })}
        {list.length === 0 && <div className="wfl-empty">No blueprint matches.</div>}
      </div>
    </div>
  );
}
