import { useState } from 'react';
import { X } from 'lucide-react';
import type { ScorecardsOverview } from '../../api/scorecards/client';
import { LEVELS } from '../../api/scorecards/logic';
import { Modal } from '../../components/ui/Modal';
import { useCreateScorecardRule } from './queries';

/** Creates a custom rule; its outcomes are reported per service from its panel or the API. */
export function RuleModal({ data, onClose }: { data: ScorecardsOverview; onClose: () => void }) {
  const [name, setName] = useState('');
  const [scorecard, setScorecard] = useState(data.scorecards[0]?.name ?? '');
  const [level, setLevel] = useState(1);
  const [scope, setScope] = useState('');
  const [description, setDescription] = useState('');
  const create = useCreateScorecardRule();

  return (
    <Modal label="New rule" onClose={onClose} width={560} height="auto">
      <div className="sc-modal">
        <header className="sc-modal-head">
          <h2>New custom rule</h2>
          <button className="sc-icon-btn" aria-label="Close" onClick={onClose}>
            <X size={18} />
          </button>
        </header>
        <div className="sc-form">
          <label className="sc-field">
            <span>Name</span>
            <input className="sc-input" autoFocus value={name} onChange={(e) => setName(e.target.value)} placeholder="Load tested this quarter" />
          </label>
          <label className="sc-field">
            <span>Scorecard</span>
            <input className="sc-input" list="sc-cards" value={scorecard} onChange={(e) => setScorecard(e.target.value)} placeholder="Pick one or name a new scorecard" />
            <datalist id="sc-cards">
              {data.scorecards.map((c) => (
                <option key={c.id} value={c.name} />
              ))}
            </datalist>
          </label>
          <div className="sc-field">
            <span>Level</span>
            <div className="btn-group">
              {LEVELS.map((l) => (
                <button key={l} className={`btn btn-sm${l === level ? ' btn-selected' : ''}`} onClick={() => setLevel(l)}>
                  Level {l}
                </button>
              ))}
            </div>
          </div>
          <label className="sc-field">
            <span>Scope</span>
            <input className="sc-input mono" value={scope} onChange={(e) => setScope(e.target.value)} placeholder="All services (e.g. team:checkout)" />
          </label>
          <label className="sc-field">
            <span>Description</span>
            <textarea className="sc-input" rows={3} value={description} onChange={(e) => setDescription(e.target.value)} placeholder="What passing this rule means" />
          </label>
          {create.isError && <div className="form-error">{(create.error as Error).message}</div>}
          <div className="sc-modal-actions">
            <button className="btn" onClick={onClose}>
              Cancel
            </button>
            <button
              className="btn btn-primary"
              disabled={!name.trim() || !scorecard.trim() || create.isPending}
              onClick={() => create.mutate({ name, scorecard, level, scope, description }, { onSuccess: onClose })}
            >
              {create.isPending ? 'Creating…' : 'Create Rule'}
            </button>
          </div>
        </div>
      </div>
    </Modal>
  );
}
