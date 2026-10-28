import { Link, useParams } from 'react-router-dom';
import { ArrowLeft, Download, EyeOff } from 'lucide-react';
import type { Survey, SurveyQuestionResult, SurveyResults } from '../../../api/portalSurveys/client';
import { responsesCsv } from '../../../api/portalSurveys/logic';
import { CopyLinkButton, fmtDate, fmtPct, StateChip } from './common';
import { useExportSurveyResponses, useSurvey, useSurveyResults } from './queries';
import './surveys.css';

/** A survey's results: response rate, a chart per question and the rate per team. */
export function SurveyResultsPage() {
  const { id = '' } = useParams();
  const survey = useSurvey(id);
  const results = useSurveyResults(id);
  if (survey.isLoading || results.isLoading) return <div className="psv-page sc-muted">Loading results…</div>;
  if (survey.isError) return <div className="psv-page form-error">{(survey.error as Error).message}</div>;
  if (results.isError) return <div className="psv-page form-error">{(results.error as Error).message}</div>;
  return <Results survey={survey.data!} results={results.data!} />;
}

function Results({ survey, results }: { survey: Survey; results: SurveyResults }) {
  const exporter = useExportSurveyResponses();
  const nps = results.questions.find((q) => q.nps);
  const download = () =>
    exporter.mutate(survey.id, {
      onSuccess: (rows) => {
        const blob = new Blob([responsesCsv(survey.questions, rows)], { type: 'text/csv;charset=utf-8' });
        const a = document.createElement('a');
        a.href = URL.createObjectURL(blob);
        a.download = `${survey.title.replace(/[^\w-]+/g, '-').toLowerCase() || 'survey'}-responses.csv`;
        a.click();
        URL.revokeObjectURL(a.href);
      },
    });

  return (
    <div className="psv-page">
      <Link to="/portal/surveys" className="psv-back">
        <ArrowLeft size={14} /> Surveys
      </Link>
      <div className="psv-results-head">
        <div>
          <h1>
            {survey.title} <StateChip state={survey.state} />
          </h1>
          <p className="sc-muted">
            {survey.audienceTeams.length ? `Asked ${survey.audienceTeams.map((t) => `@${t}`).join(', ')}` : 'Asked every member'}
            {survey.closesAt != null && ` · closes ${fmtDate(survey.closesAt)}`}
            {survey.anonymous && (
              <span className="psv-anon">
                <EyeOff size={12} /> Anonymous
              </span>
            )}
          </p>
        </div>
        <div className="psv-results-actions">
          <CopyLinkButton id={survey.id} className="btn" />
          <Link className="btn" to={`/portal/surveys/${survey.id}/edit`}>
            Edit
          </Link>
          <button className="btn" onClick={download} disabled={exporter.isPending || !results.responses}>
            <Download size={14} /> Export CSV
          </button>
        </div>
      </div>
      {exporter.isError && <div className="form-error">{(exporter.error as Error).message}</div>}

      <div className="psv-kpis">
        <Kpi label="Responses" value={String(results.responses)} />
        <Kpi label="Response rate" value={fmtPct(results.responseRate)} sub={`of ${results.audienceSize} members asked`} />
        {nps?.nps && <Kpi label="NPS" value={formatNps(nps.nps.score)} sub={`${nps.nps.promoters} promoters · ${nps.nps.detractors} detractors`} tone={nps.nps.score >= 0 ? 'good' : 'bad'} />}
        <Kpi label="Questions" value={String(results.questions.length)} />
      </div>

      {!results.responses && <div className="psv-note">No responses yet. Share the survey link with the people you asked.</div>}

      <div className="psv-results-grid">
        <div className="psv-results-questions">
          {results.questions.map((q, i) => (
            <QuestionResult key={q.questionId} index={i} result={q} />
          ))}
        </div>
        <aside className="psv-card psv-team-rates">
          <h3>Response rate by team</h3>
          {results.teams.length === 0 && <p className="sc-muted">No teams in the audience.</p>}
          {results.teams.map((t) => (
            <div key={t.team} className="psv-team-rate">
              <div className="psv-team-rate-label">
                <span>{t.name}</span>
                <span className="sc-muted">
                  {t.responded}/{t.members}
                </span>
              </div>
              <span className="psv-rate-track wide">
                <span className="psv-rate-fill" style={{ width: `${t.rate ?? 0}%` }} />
              </span>
              <span className="psv-team-rate-pct">{fmtPct(t.rate)}</span>
            </div>
          ))}
        </aside>
      </div>
    </div>
  );
}

const formatNps = (score: number) => `${score > 0 ? '+' : ''}${Math.round(score)}`;

function Kpi({ label, value, sub, tone }: { label: string; value: string; sub?: string; tone?: 'good' | 'bad' }) {
  return (
    <div className="psv-kpi">
      <span className="psv-kpi-label">{label}</span>
      <span className={`psv-kpi-value${tone ? ` ${tone}` : ''}`}>{value}</span>
      {sub && <span className="psv-kpi-sub">{sub}</span>}
    </div>
  );
}

function QuestionResult({ index, result: q }: { index: number; result: SurveyQuestionResult }) {
  const max = Math.max(1, ...q.counts.map((c) => c.count));
  const total = q.kind === 'multi' ? q.answered : q.counts.reduce((a, c) => a + c.count, 0);
  return (
    <section className="psv-card psv-qresult">
      <header className="psv-qresult-head">
        <h3>
          {index + 1}. {q.title}
        </h3>
        <span className="sc-muted">{q.answered} answered</span>
      </header>
      {q.kind === 'likert' && q.average != null && (
        <div className="psv-qresult-stat">
          Average <strong>{q.average.toFixed(2)}</strong> / 5
        </div>
      )}
      {q.kind === 'nps' && q.nps && <NpsBar nps={q.nps} />}
      {q.kind === 'nps' ? (
        <div className="psv-columns" role="img" aria-label="Answers from 0 to 10">
          {q.counts.map((c) => (
            <div key={c.label} className="psv-column" title={`${c.label}: ${c.count}`}>
              <span className="psv-column-count">{c.count || ''}</span>
              <span className={`psv-column-bar ${Number(c.label) >= 9 ? 'promoter' : Number(c.label) >= 7 ? 'passive' : 'detractor'}`} style={{ height: `${(c.count * 100) / max}%` }} />
              <span className="psv-column-label">{c.label}</span>
            </div>
          ))}
        </div>
      ) : q.kind === 'text' ? (
        <ul className="psv-texts">
          {q.texts.map((t, i) => (
            <li key={i}>{t}</li>
          ))}
          {!q.texts.length && <li className="sc-muted">No answers yet.</li>}
        </ul>
      ) : (
        <div className={`psv-bars${q.kind === 'likert' ? ' likert' : ''}`}>
          {q.counts.map((c, i) => (
            <div key={c.label} className="psv-bar-row">
              <span className="psv-bar-label">{c.label}</span>
              <span className="psv-bar-track">
                <span className={`psv-bar-fill${q.kind === 'likert' ? ` l${i + 1}` : ''}`} style={{ width: `${(c.count * 100) / max}%` }} />
              </span>
              <span className="psv-bar-value">
                {c.count} <span className="sc-muted">{total ? `${Math.round((c.count * 100) / total)}%` : ''}</span>
              </span>
            </div>
          ))}
        </div>
      )}
    </section>
  );
}

function NpsBar({ nps }: { nps: NonNullable<SurveyQuestionResult['nps']> }) {
  const total = nps.promoters + nps.passives + nps.detractors;
  const pct = (n: number) => `${(n * 100) / total}%`;
  return (
    <div className="psv-nps-summary">
      <div className="psv-qresult-stat">
        NPS <strong>{formatNps(nps.score)}</strong>
      </div>
      <div className="psv-nps-stack" title={`${nps.detractors} detractors · ${nps.passives} passives · ${nps.promoters} promoters`}>
        <span className="detractor" style={{ width: pct(nps.detractors) }} />
        <span className="passive" style={{ width: pct(nps.passives) }} />
        <span className="promoter" style={{ width: pct(nps.promoters) }} />
      </div>
      <div className="psv-nps-legend">
        <span>
          <i className="detractor" /> Detractors (0–6) {nps.detractors}
        </span>
        <span>
          <i className="passive" /> Passives (7–8) {nps.passives}
        </span>
        <span>
          <i className="promoter" /> Promoters (9–10) {nps.promoters}
        </span>
      </div>
    </div>
  );
}
