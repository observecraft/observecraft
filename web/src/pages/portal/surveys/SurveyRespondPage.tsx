import { useState } from 'react';
import { Link, useParams } from 'react-router-dom';
import { CheckCircle2, EyeOff } from 'lucide-react';
import type { SurveyAnswers, SurveyForm, SurveyQuestion } from '../../../api/portalSurveys/client';
import { checkAnswers, LIKERT_LABELS, NPS_SCALE } from '../../../api/portalSurveys/logic';
import { fmtDate } from './common';
import { useSubmitSurveyResponse, useSurveyForm } from './queries';
import './surveys.css';

/** The form a member fills in, opened from the survey's link. */
export function SurveyRespondPage() {
  const { id = '' } = useParams();
  const form = useSurveyForm(id);
  if (form.isLoading) return <Frame>Loading survey…</Frame>;
  if (form.isError) return <Frame>{(form.error as Error).message}</Frame>;
  return <Respond form={form.data!} />;
}

function Frame({ children }: { children: React.ReactNode }) {
  return (
    <div className="psv-respond">
      <div className="psv-respond-card psv-respond-message">{children}</div>
    </div>
  );
}

function Respond({ form }: { form: SurveyForm }) {
  const [answers, setAnswers] = useState<SurveyAnswers>({});
  const [error, setError] = useState<string | null>(null);
  const [done, setDone] = useState(false);
  const submit = useSubmitSurveyResponse();

  if (done || form.responded)
    return (
      <Frame>
        <CheckCircle2 size={36} className="psv-respond-ok" />
        <h2>Thanks for your response</h2>
        <p className="sc-muted">{done ? 'Your answers were recorded.' : 'You have already responded to this survey.'}</p>
        <Link to="/portal/surveys" className="btn">
          Back to surveys
        </Link>
      </Frame>
    );
  if (form.state !== 'open')
    return (
      <Frame>
        <h2>{form.title}</h2>
        <p className="sc-muted">{form.state === 'scheduled' ? "This survey hasn't opened yet." : "This survey isn't accepting responses."}</p>
      </Frame>
    );
  if (!form.inAudience)
    return (
      <Frame>
        <h2>{form.title}</h2>
        <p className="sc-muted">This survey is for members of other teams.</p>
      </Frame>
    );

  const set = (qid: string, v: unknown) => setAnswers((a) => ({ ...a, [qid]: v }));
  const onSubmit = (e: React.FormEvent) => {
    e.preventDefault();
    setError(null);
    let checked: SurveyAnswers;
    try {
      checked = checkAnswers(form.questions, answers);
    } catch (err) {
      setError((err as Error).message);
      return;
    }
    submit.mutate({ id: form.id, input: { answers: checked } }, { onSuccess: () => setDone(true), onError: (err) => setError((err as Error).message) });
  };

  return (
    <div className="psv-respond">
      <form className="psv-respond-card" onSubmit={onSubmit}>
        <div className="psv-respond-bar" />
        <header className="psv-respond-head">
          <h1>{form.title}</h1>
          {form.description && <p>{form.description}</p>}
          <div className="psv-respond-meta">
            {form.anonymous && (
              <span className="psv-anon">
                <EyeOff size={13} /> Anonymous: your name isn't stored with your answers
              </span>
            )}
            {form.closesAt != null && <span className="sc-muted">Open until {fmtDate(form.closesAt)}</span>}
          </div>
        </header>
        {form.questions.map((q, i) => (
          <fieldset key={q.id} className="psv-respond-q">
            <legend>
              {i + 1}. {q.title}
              {q.required && (
                <span className="psv-required-mark" aria-label="required">
                  *
                </span>
              )}
            </legend>
            {q.description && <p className="psv-hint">{q.description}</p>}
            <AnswerInput question={q} value={answers[q.id]} onChange={(v) => set(q.id, v)} />
          </fieldset>
        ))}
        {error && <div className="form-error">{error}</div>}
        <div className="psv-respond-foot">
          <button className="btn btn-primary" type="submit" disabled={submit.isPending}>
            {submit.isPending ? 'Submitting…' : 'Submit'}
          </button>
        </div>
      </form>
    </div>
  );
}

function AnswerInput({ question: q, value, onChange }: { question: SurveyQuestion; value: unknown; onChange: (v: unknown) => void }) {
  const name = `q-${q.id}`;
  if (q.kind === 'likert')
    return (
      <div className="psv-likert" role="radiogroup" aria-label={q.title}>
        {LIKERT_LABELS.map((label, i) => (
          <label key={label} className={`psv-likert-option${value === i + 1 ? ' on' : ''}`}>
            <input type="radio" name={name} checked={value === i + 1} onChange={() => onChange(i + 1)} />
            <span className="psv-likert-dot" />
            <span>{label}</span>
          </label>
        ))}
      </div>
    );
  if (q.kind === 'nps')
    return (
      <div>
        <div className="psv-nps" role="radiogroup" aria-label={q.title}>
          {NPS_SCALE.map((n) => (
            <button type="button" key={n} role="radio" aria-checked={value === n} className={`psv-nps-option${value === n ? ' on' : ''}`} onClick={() => onChange(n)}>
              {n}
            </button>
          ))}
        </div>
        <div className="psv-nps-ends">
          <span>Not at all likely</span>
          <span>Extremely likely</span>
        </div>
      </div>
    );
  if (q.kind === 'single')
    return (
      <div className="psv-choices">
        {q.options.map((o) => (
          <label key={o} className="psv-radio">
            <input type="radio" name={name} checked={value === o} onChange={() => onChange(o)} /> {o}
          </label>
        ))}
      </div>
    );
  if (q.kind === 'multi') {
    const picked = Array.isArray(value) ? (value as string[]) : [];
    return (
      <div className="psv-choices">
        {q.options.map((o) => (
          <label key={o} className="psv-check">
            <input type="checkbox" checked={picked.includes(o)} onChange={(e) => onChange(e.target.checked ? [...picked, o] : picked.filter((p) => p !== o))} /> {o}
          </label>
        ))}
      </div>
    );
  }
  return <textarea className="sc-input psv-respond-text" rows={4} value={typeof value === 'string' ? value : ''} onChange={(e) => onChange(e.target.value)} maxLength={5000} aria-label={q.title} />;
}
