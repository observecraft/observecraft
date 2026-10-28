import { useState } from 'react';
import { Link, useNavigate } from 'react-router-dom';
import { ArrowLeft, ArrowRight, ChevronRight, LayoutTemplate, ListPlus, Sparkles } from 'lucide-react';
import { emptyTrigger, TRIGGER_LABEL } from '../../../api/workflows/logic';
import type { WorkflowInput, WorkflowTriggerKind } from '../../../api/workflows/types';
import { blueprintByKey, matchBlueprints, type WorkflowBlueprint } from './blueprints';
import { LogoChip, TRIGGER_ICON } from './common';
import { useCreateDraft } from './useStartFromBlueprint';
import './workflows.css';

const SUGGESTIONS: { logos: string[]; text: string; blueprint: string; tweak?: (w: WorkflowInput) => void }[] = [
  {
    logos: ['github', 'slack'],
    text: 'Every day, find pull requests open for more than 2 weeks and post them to #storefront',
    blueprint: 'weekly_pr_digest',
    tweak: (w) => {
      w.spec.triggers[0].cron = '0 9 * * *';
      w.name = 'Daily stale pull requests';
    },
  },
  { logos: ['schedule', 'slack'], text: 'Daily report of the top 3 services by error rate, posted to #checkout', blueprint: 'daily_error_report' },
  { logos: ['monitor', 'slack'], text: 'When an error-rate monitor alerts, check again, restart the service and tell the team', blueprint: 'restart_on_alert' },
];

const TRIGGERS: WorkflowTriggerKind[] = ['monitor', 'schedule', 'incident', 'api', 'manual'];

function blank(name: string, trigger: WorkflowTriggerKind): WorkflowInput {
  return { name, handle: '', description: '', tags: [], ownerTeam: '', service: '', blueprint: '', spec: { inputs: [], triggers: [emptyTrigger(trigger)], steps: [] } };
}

/** /automation/workflows/new: describe what to automate, or start from a trigger, an action or a blueprint. */
export function NewWorkflowPage() {
  const navigate = useNavigate();
  const create = useCreateDraft();
  const [prompt, setPrompt] = useState('');
  const matches = matchBlueprints(prompt);
  const fromBlueprint = (b: WorkflowBlueprint, tweak?: (w: WorkflowInput) => void, description?: string) => {
    const w = b.build();
    tweak?.(w);
    if (description) w.description = description;
    create(w);
  };
  const generate = () => {
    const text = prompt.trim();
    if (!text) return;
    const best = matches[0];
    if (best) fromBlueprint(best, undefined, text);
    else create({ ...blank(text.length > 80 ? `${text.slice(0, 77)}…` : text, 'manual'), description: text });
  };
  return (
    <div className="wfn-page">
      <header className="wfn-top">
        <button className="wf-icon-btn" aria-label="Back" onClick={() => navigate('/automation')}>
          <ArrowLeft size={18} />
        </button>
        <span className="wfn-crumb">
          <Link to="/automation">Workflow Automation</Link> / New workflow
        </span>
      </header>
      <div className="wfn-body">
        <h1>Build a workflow</h1>
        <div className="wfn-prompt">
          <textarea
            placeholder="What do you want to automate? e.g. “When the billing SLO burns, page payments and declare an incident”"
            value={prompt}
            onChange={(e) => setPrompt(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === 'Enter' && (e.metaKey || e.ctrlKey)) generate();
            }}
          />
          <div className="wfn-prompt-foot">
            <span className="wf-muted">
              {prompt.trim()
                ? matches.length
                  ? `Closest blueprint: ${matches[0].title}`
                  : 'No blueprint matches: a blank workflow with a manual trigger is created.'
                : 'Drafts are made from the blueprints whose keywords match your words. Nothing is sent anywhere.'}
            </span>
            <button className="btn btn-primary" disabled={!prompt.trim() || create.pending} onClick={generate}>
              <Sparkles size={14} /> Create draft
            </button>
          </div>
        </div>
        {create.error && <div className="wf-error">{create.error.message}</div>}
        <div className="wfn-suggestions">
          {SUGGESTIONS.map((s) => (
            <button
              key={s.text}
              className="wfn-suggestion"
              disabled={create.pending}
              onClick={() => {
                const b = blueprintByKey(s.blueprint);
                if (b) fromBlueprint(b, s.tweak, s.text);
              }}
            >
              <span className="wfn-logos">
                {s.logos.map((l) => (
                  <LogoChip key={l} logo={l} size={26} />
                ))}
              </span>
              <span className="wfn-suggestion-text">{s.text}</span>
              <ArrowRight size={14} className="wfn-go" />
            </button>
          ))}
        </div>
        <div className="wfn-start">
          <div className="wfn-start-head">Or start with a</div>
          <div className="wfn-start-trigger">
            <div className="wfn-start-label">Trigger</div>
            <div className="wfn-chips">
              {TRIGGERS.map((k) => (
                <button key={k} className="wfn-chip" disabled={create.pending} onClick={() => create(blank(`New ${TRIGGER_LABEL[k].toLowerCase()} workflow`, k), 'trigger')}>
                  {TRIGGER_ICON[k]} {TRIGGER_LABEL[k]}
                </button>
              ))}
              <button className="wfn-chip wfn-chip-more" disabled={create.pending} onClick={() => create(blank('New app workflow', 'app'), 'trigger')}>
                View all <ChevronRight size={13} />
              </button>
            </div>
          </div>
          <div className="wfn-start-cards">
            <button className="wfn-card" disabled={create.pending} onClick={() => create(blank('New workflow', 'manual'), 'catalog')}>
              <span className="wfn-card-icon">
                <ListPlus size={20} />
              </span>
              <span>
                <span className="wfn-card-title">Action</span>
                <span className="wfn-card-sub">Choose from the action catalog</span>
              </span>
            </button>
            <Link to="/automation/blueprints" className="wfn-card">
              <span className="wfn-card-icon">
                <LayoutTemplate size={20} />
              </span>
              <span>
                <span className="wfn-card-title">Blueprint</span>
                <span className="wfn-card-sub">Use a pre-built template</span>
              </span>
            </Link>
          </div>
        </div>
      </div>
    </div>
  );
}
