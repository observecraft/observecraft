import { useState } from 'react';
import { Link, useNavigate } from 'react-router-dom';
import { EyeOff, Plus, Search, Trash2 } from 'lucide-react';
import type { Survey } from '../../../api/portalSurveys/client';
import { CopyLinkButton, fmtDate, fmtPct, StateChip, TemplatePicker } from './common';
import { useDeleteSurvey, useSurveys } from './queries';
import { SurveysIllustration } from './SurveysIllustration';
import './surveys.css';

/** Developer Portal → Surveys: the first-run screen, then the list of surveys. */
export function SurveysPage() {
  const surveys = useSurveys();
  const [picking, setPicking] = useState(false);

  if (surveys.isLoading) return <div className="psv-page sc-muted">Loading surveys…</div>;
  if (surveys.isError) return <div className="psv-page form-error">{(surveys.error as Error).message}</div>;
  const list = surveys.data ?? [];

  return (
    <div className="psv-page">
      {list.length === 0 ? (
        <section className="psv-empty">
          <h1>Create Your First Survey</h1>
          <p>Start from a template or create a custom survey to gather feedback from your developers</p>
          <button className="btn btn-primary psv-empty-cta" onClick={() => setPicking(true)}>
            <Plus size={15} /> Create New Survey
          </button>
          <SurveysIllustration />
        </section>
      ) : (
        <SurveyList surveys={list} onCreate={() => setPicking(true)} />
      )}
      {picking && <TemplatePicker onClose={() => setPicking(false)} />}
    </div>
  );
}

function SurveyList({ surveys, onCreate }: { surveys: Survey[]; onCreate: () => void }) {
  const [query, setQuery] = useState('');
  const navigate = useNavigate();
  const remove = useDeleteSurvey();
  const q = query.trim().toLowerCase();
  const shown = surveys.filter((s) => !q || s.title.toLowerCase().includes(q) || s.description.toLowerCase().includes(q));

  const onDelete = (s: Survey) => {
    if (window.confirm(`Delete “${s.title}” and its ${s.responses} responses?`)) remove.mutate(s.id);
  };

  return (
    <>
      <div className="psv-list-head">
        <div>
          <h1>Surveys</h1>
          <p className="sc-muted">Ask developers how building and shipping feels, and track the answers over time.</p>
        </div>
        <button className="btn btn-primary" onClick={onCreate}>
          <Plus size={15} /> New Survey
        </button>
      </div>
      <label className="psv-search">
        <Search size={14} />
        <input value={query} onChange={(e) => setQuery(e.target.value)} placeholder="Search surveys…" aria-label="Search surveys" />
      </label>
      {remove.isError && <div className="form-error">{(remove.error as Error).message}</div>}
      <table className="psv-table">
        <thead>
          <tr>
            <th>Survey</th>
            <th>Status</th>
            <th>Responses</th>
            <th>Response rate</th>
            <th>Closes</th>
            <th>Created by</th>
            <th aria-label="Actions" />
          </tr>
        </thead>
        <tbody>
          {shown.map((s) => {
            const rate = s.audienceSize ? (Math.min(s.responses, s.audienceSize) * 100) / s.audienceSize : null;
            return (
              <tr key={s.id} onClick={() => navigate(`/portal/surveys/${s.id}/results`)}>
                <td>
                  <Link className="psv-table-title" to={`/portal/surveys/${s.id}/results`} onClick={(e) => e.stopPropagation()}>
                    {s.title}
                  </Link>
                  <div className="psv-table-sub">
                    {s.questions.length} questions · {s.audienceTeams.length ? s.audienceTeams.map((t) => `@${t}`).join(', ') : 'All members'}
                    {s.anonymous && (
                      <span className="psv-anon">
                        <EyeOff size={12} /> Anonymous
                      </span>
                    )}
                  </div>
                </td>
                <td>
                  <StateChip state={s.state} />
                </td>
                <td>
                  {s.responses} <span className="sc-muted">/ {s.audienceSize}</span>
                </td>
                <td>
                  <span className="psv-rate">
                    <span className="psv-rate-track">
                      <span className="psv-rate-fill" style={{ width: `${rate ?? 0}%` }} />
                    </span>
                    {fmtPct(rate)}
                  </span>
                </td>
                <td>{fmtDate(s.closesAt)}</td>
                <td>{s.createdBy || '—'}</td>
                <td className="psv-row-actions" onClick={(e) => e.stopPropagation()}>
                  {s.state === 'open' && !s.responded && (
                    <Link className="btn btn-sm" to={`/portal/surveys/${s.id}/respond`}>
                      Respond
                    </Link>
                  )}
                  <CopyLinkButton id={s.id} />
                  <Link className="btn btn-sm" to={`/portal/surveys/${s.id}/edit`}>
                    Edit
                  </Link>
                  <button className="sc-icon-btn" aria-label={`Delete ${s.title}`} onClick={() => onDelete(s)}>
                    <Trash2 size={14} />
                  </button>
                </td>
              </tr>
            );
          })}
          {!shown.length && (
            <tr>
              <td colSpan={7} className="psv-table-empty">
                No survey matches “{query}”.
              </td>
            </tr>
          )}
        </tbody>
      </table>
    </>
  );
}
