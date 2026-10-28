import { useState } from 'react';
import { useNavigate } from 'react-router-dom';
import { ClipboardList, FilePlus2, X } from 'lucide-react';
import type { SurveyState } from '../../../api/portalSurveys/client';
import { STATE_LABEL } from '../../../api/portalSurveys/logic';
import { SURVEY_TEMPLATES } from '../../../api/portalSurveys/templates';
import { Modal } from '../../../components/ui/Modal';

export function StateChip({ state }: { state: SurveyState }) {
  return <span className={`psv-state psv-state-${state}`}>{STATE_LABEL[state]}</span>;
}

/** The in-app link respondents open. */
export const respondUrl = (id: string) => `${window.location.origin}/portal/surveys/${id}/respond`;

/** Copies the respond link; the label confirms for a moment. */
export function CopyLinkButton({ id, className = 'btn btn-sm' }: { id: string; className?: string }) {
  const [copied, setCopied] = useState(false);
  const copy = async () => {
    try {
      await navigator.clipboard.writeText(respondUrl(id));
      setCopied(true);
      window.setTimeout(() => setCopied(false), 1500);
    } catch {
      window.prompt('Copy the survey link', respondUrl(id));
    }
  };
  return (
    <button className={className} onClick={copy}>
      {copied ? 'Link copied' : 'Copy link'}
    </button>
  );
}

/** Pick a template or start from scratch; opens the builder. */
export function TemplatePicker({ onClose }: { onClose: () => void }) {
  const navigate = useNavigate();
  return (
    <Modal label="Create a survey" onClose={onClose} width={760} height="auto">
      <div className="psv-picker">
        <header className="psv-picker-head">
          <h2>Create a survey</h2>
          <button className="sc-icon-btn" aria-label="Close" onClick={onClose}>
            <X size={18} />
          </button>
        </header>
        <p className="psv-picker-lede">Start from a template written for engineering teams, or build your own questions.</p>
        <div className="psv-picker-grid">
          {SURVEY_TEMPLATES.map((t) => (
            <button key={t.key} className="psv-template" onClick={() => navigate(`/portal/surveys/new?template=${t.key}`)}>
              <ClipboardList size={20} className="psv-template-icon" />
              <span className="psv-template-title">{t.title}</span>
              <span className="psv-template-summary">{t.summary}</span>
              <span className="psv-template-count">{t.questions.length} questions</span>
            </button>
          ))}
          <button className="psv-template blank" onClick={() => navigate('/portal/surveys/new')}>
            <FilePlus2 size={20} className="psv-template-icon" />
            <span className="psv-template-title">Custom survey</span>
            <span className="psv-template-summary">Start with an empty survey and add the questions you need.</span>
          </button>
        </div>
      </div>
    </Modal>
  );
}

export const fmtDate = (ms: number | null | undefined) => (ms == null ? '—' : new Date(ms).toLocaleDateString(undefined, { year: 'numeric', month: 'short', day: 'numeric' }));
export const fmtPct = (v: number | null | undefined) => (v == null ? '—' : `${Math.round(v)}%`);
