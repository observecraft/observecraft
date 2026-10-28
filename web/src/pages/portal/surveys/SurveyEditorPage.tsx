import { useEffect, useState } from 'react';
import { Link, useNavigate, useParams, useSearchParams } from 'react-router-dom';
import { useQuery } from '@tanstack/react-query';
import { ArrowDown, ArrowLeft, ArrowUp, Plus, Trash2, X } from 'lucide-react';
import { API_MODE } from '../../../api/http';
import { listChannels } from '../../../api/notifications/client';
import type { SurveyInput, SurveyQuestion, SurveyQuestionKind, SurveyStatus } from '../../../api/portalSurveys/client';
import { blankQuestion, checkSurvey, KIND_LABEL, LIKERT_LABELS } from '../../../api/portalSurveys/logic';
import { templateByKey } from '../../../api/portalSurveys/templates';
import { useTeams } from '../../settings/queries';
import { useCreateSurvey, useSurvey, useUpdateSurvey } from './queries';
import './surveys.css';

const KINDS: SurveyQuestionKind[] = ['likert', 'single', 'multi', 'nps', 'text'];
/** Suggested recipients in mock mode (live mode lists the org's notification channels). */
const MOCK_CHANNELS = ['slack-devex', 'slack-platform', 'webhook-portal'];

function emptySurvey(template: string | null): SurveyInput {
  const t = templateByKey(template);
  return {
    title: t?.title ?? '',
    description: t?.description ?? '',
    template: t?.key ?? '',
    questions: t ? structuredClone(t.questions) : [{ ...blankQuestion('likert', []), required: true }],
    audienceTeams: [],
    anonymous: false,
    status: 'draft',
    opensAt: null,
    closesAt: null,
    notify: [],
  };
}

/** epoch ms ↔ the value of a datetime-local input, in the browser's zone. */
const toLocal = (ms: number | null | undefined) => (ms == null ? '' : new Date(ms - new Date(ms).getTimezoneOffset() * 60_000).toISOString().slice(0, 16));
const fromLocal = (v: string) => (v ? new Date(v).getTime() : null);

/** Builds a survey: questions, audience, dates, anonymity and who hears about responses. */
export function SurveyEditorPage() {
  const { id } = useParams();
  const [params] = useSearchParams();
  const navigate = useNavigate();
  const existing = useSurvey(id);
  const [draft, setDraft] = useState<SurveyInput | null>(id ? null : () => emptySurvey(params.get('template')));
  const [error, setError] = useState<string | null>(null);
  const create = useCreateSurvey();
  const update = useUpdateSurvey();
  const teams = useTeams({ limit: 200, offset: 0, sort: 'name' });
  const channels = useQuery({ queryKey: ['notification-channels'], queryFn: listChannels, enabled: API_MODE === 'live', retry: 1 });

  useEffect(() => {
    if (id && existing.data && !draft) {
      const { title, description, template, questions, audienceTeams, anonymous, status, opensAt, closesAt, notify } = existing.data;
      setDraft({ title, description, template, questions, audienceTeams, anonymous, status, opensAt: opensAt ?? null, closesAt: closesAt ?? null, notify });
    }
  }, [id, existing.data, draft]);

  if (id && existing.isError) return <div className="psv-page form-error">{(existing.error as Error).message}</div>;
  if (!draft) return <div className="psv-page sc-muted">Loading survey…</div>;
  const locked = !!existing.data?.responses;
  const set = (patch: Partial<SurveyInput>) => setDraft((d) => (d ? { ...d, ...patch } : d));
  const setQuestion = (i: number, patch: Partial<SurveyQuestion>) => set({ questions: draft.questions.map((q, j) => (j === i ? { ...q, ...patch } : q)) });
  const move = (i: number, by: number) => {
    const qs = [...draft.questions];
    const [q] = qs.splice(i, 1);
    qs.splice(i + by, 0, q);
    set({ questions: qs });
  };
  const channelHandles = API_MODE === 'live' ? (channels.data ?? []).map((c) => c.handle) : MOCK_CHANNELS;

  const save = (status: SurveyStatus) => {
    setError(null);
    let input: SurveyInput;
    try {
      input = checkSurvey({ ...draft, status });
    } catch (e) {
      setError((e as Error).message);
      return;
    }
    const done = (s: { id: string }) => navigate(status === 'draft' ? '/portal/surveys' : `/portal/surveys/${s.id}/results`);
    const onError = (e: Error) => setError(e.message);
    if (id) update.mutate({ id, input }, { onSuccess: done, onError });
    else create.mutate(input, { onSuccess: done, onError });
  };
  const pending = create.isPending || update.isPending;

  return (
    <div className="psv-page psv-editor">
      <div className="psv-editor-top">
        <Link to="/portal/surveys" className="psv-back">
          <ArrowLeft size={14} /> Surveys
        </Link>
        <h1>{id ? 'Edit survey' : 'New survey'}</h1>
      </div>
      <div className="psv-editor-grid">
        <div className="psv-editor-main">
          <section className="psv-card">
            <label className="sc-field">
              <span>Title</span>
              <input className="sc-input psv-title-input" value={draft.title} onChange={(e) => set({ title: e.target.value })} placeholder="Developer Experience Survey" maxLength={200} />
            </label>
            <label className="sc-field">
              <span>Description</span>
              <textarea
                className="sc-input"
                rows={2}
                value={draft.description}
                onChange={(e) => set({ description: e.target.value })}
                placeholder="What the survey is about and how answers are used"
              />
            </label>
          </section>

          {locked && <div className="psv-note">This survey has responses: you can reword questions, but not add, remove or change their kind and options.</div>}

          {draft.questions.map((q, i) => (
            <QuestionCard
              key={q.id}
              index={i}
              question={q}
              locked={locked}
              last={i === draft.questions.length - 1}
              onChange={(patch) => setQuestion(i, patch)}
              onMove={(by) => move(i, by)}
              onRemove={() => set({ questions: draft.questions.filter((_, j) => j !== i) })}
            />
          ))}

          {!locked && (
            <div className="psv-add">
              <span className="sc-muted">Add a question:</span>
              {KINDS.map((k) => (
                <button key={k} className="btn btn-sm" onClick={() => set({ questions: [...draft.questions, blankQuestion(k, draft.questions)] })}>
                  <Plus size={13} /> {KIND_LABEL[k]}
                </button>
              ))}
            </div>
          )}
        </div>

        <aside className="psv-editor-side">
          <section className="psv-card">
            <h3>Audience</h3>
            <label className="psv-radio">
              <input type="radio" checked={!draft.audienceTeams.length} onChange={() => set({ audienceTeams: [] })} /> Every member of the organization
            </label>
            <label className="psv-radio">
              <input type="radio" checked={draft.audienceTeams.length > 0} onChange={() => set({ audienceTeams: teams.data?.teams[0] ? [teams.data.teams[0].handle] : [] })} /> Members of specific
              teams
            </label>
            {draft.audienceTeams.length > 0 && (
              <div className="psv-team-list">
                {(teams.data?.teams ?? []).map((t) => (
                  <label key={t.handle} className="psv-check">
                    <input
                      type="checkbox"
                      checked={draft.audienceTeams.includes(t.handle)}
                      onChange={(e) => set({ audienceTeams: e.target.checked ? [...draft.audienceTeams, t.handle] : draft.audienceTeams.filter((h) => h !== t.handle) })}
                    />
                    {t.name} <span className="sc-muted">@{t.handle}</span>
                  </label>
                ))}
              </div>
            )}
          </section>

          <section className="psv-card">
            <h3>Responses</h3>
            <label className="psv-check">
              <input type="checkbox" checked={draft.anonymous} disabled={locked} onChange={(e) => set({ anonymous: e.target.checked })} /> Anonymous
            </label>
            <p className="psv-hint">Anonymous responses keep no name or email. Each member can still respond only once.</p>
            <div className="psv-dates">
              <label className="sc-field">
                <span>Opens</span>
                <input type="datetime-local" className="sc-input" value={toLocal(draft.opensAt)} onChange={(e) => set({ opensAt: fromLocal(e.target.value) })} />
              </label>
              <label className="sc-field">
                <span>Closes</span>
                <input type="datetime-local" className="sc-input" value={toLocal(draft.closesAt)} onChange={(e) => set({ closesAt: fromLocal(e.target.value) })} />
              </label>
            </div>
            <p className="psv-hint">Leave empty to open when published and keep it open until you close it.</p>
          </section>

          <section className="psv-card">
            <h3>On submission</h3>
            <p className="psv-hint">Notify a channel or a member each time someone responds. Answers aren't included in the message.</p>
            <Recipients value={draft.notify} suggestions={channelHandles} onChange={(notify) => set({ notify })} />
          </section>

          {error && <div className="form-error">{error}</div>}
          <div className="psv-save">
            <button className="btn" disabled={pending} onClick={() => save(draft.status === 'draft' ? 'draft' : draft.status)}>
              {draft.status === 'draft' ? 'Save draft' : 'Save'}
            </button>
            {draft.status !== 'open' && (
              <button className="btn btn-primary" disabled={pending} onClick={() => save('open')}>
                {draft.status === 'closed' ? 'Reopen' : 'Publish'}
              </button>
            )}
            {draft.status === 'open' && (
              <button className="btn" disabled={pending} onClick={() => save('closed')}>
                Close survey
              </button>
            )}
          </div>
        </aside>
      </div>
    </div>
  );
}

function QuestionCard({
  index,
  question: q,
  locked,
  last,
  onChange,
  onMove,
  onRemove,
}: {
  index: number;
  question: SurveyQuestion;
  locked: boolean;
  last: boolean;
  onChange: (patch: Partial<SurveyQuestion>) => void;
  onMove: (by: number) => void;
  onRemove: () => void;
}) {
  const choice = q.kind === 'single' || q.kind === 'multi';
  const setOption = (i: number, value: string) => onChange({ options: q.options.map((o, j) => (j === i ? value : o)) });
  return (
    <section className="psv-card psv-question">
      <header className="psv-question-head">
        <span className="psv-question-num">{index + 1}</span>
        <select
          className="sc-input psv-kind"
          value={q.kind}
          disabled={locked}
          aria-label="Question type"
          onChange={(e) => {
            const kind = e.target.value as SurveyQuestionKind;
            const opts = kind === 'single' || kind === 'multi' ? (q.options.length ? q.options : ['Option 1', 'Option 2']) : [];
            onChange({ kind, options: opts });
          }}
        >
          {KINDS.map((k) => (
            <option key={k} value={k}>
              {KIND_LABEL[k]}
            </option>
          ))}
        </select>
        <label className="psv-check psv-required">
          <input type="checkbox" checked={q.required} onChange={(e) => onChange({ required: e.target.checked })} /> Required
        </label>
        <span className="psv-question-tools">
          <button className="sc-icon-btn" aria-label="Move up" disabled={index === 0} onClick={() => onMove(-1)}>
            <ArrowUp size={14} />
          </button>
          <button className="sc-icon-btn" aria-label="Move down" disabled={last} onClick={() => onMove(1)}>
            <ArrowDown size={14} />
          </button>
          {!locked && (
            <button className="sc-icon-btn" aria-label="Remove question" onClick={onRemove}>
              <Trash2 size={14} />
            </button>
          )}
        </span>
      </header>
      <input className="sc-input psv-question-title" value={q.title} onChange={(e) => onChange({ title: e.target.value })} placeholder="Question" aria-label={`Question ${index + 1}`} />
      <input className="sc-input" value={q.description} onChange={(e) => onChange({ description: e.target.value })} placeholder="Help text (optional)" aria-label="Help text" />
      {choice && (
        <div className="psv-options">
          {q.options.map((o, i) => (
            <div key={i} className="psv-option-row">
              <span className={`psv-option-mark ${q.kind}`} />
              <input className="sc-input" value={o} disabled={locked} onChange={(e) => setOption(i, e.target.value)} aria-label={`Option ${i + 1}`} />
              {!locked && q.options.length > 2 && (
                <button className="sc-icon-btn" aria-label="Remove option" onClick={() => onChange({ options: q.options.filter((_, j) => j !== i) })}>
                  <X size={14} />
                </button>
              )}
            </div>
          ))}
          {!locked && q.options.length < 20 && (
            <button className="sc-link psv-add-option" onClick={() => onChange({ options: [...q.options, `Option ${q.options.length + 1}`] })}>
              + Add option
            </button>
          )}
        </div>
      )}
      {q.kind === 'likert' && (
        <div className="psv-preview-scale">
          {LIKERT_LABELS.map((l) => (
            <span key={l}>{l}</span>
          ))}
        </div>
      )}
      {q.kind === 'nps' && (
        <div className="psv-preview-scale nps">
          {Array.from({ length: 11 }, (_, i) => (
            <span key={i}>{i}</span>
          ))}
        </div>
      )}
      {q.kind === 'text' && <div className="psv-preview-text">Respondents write their answer here</div>}
    </section>
  );
}

/** Channel handles and member emails as removable chips. */
function Recipients({ value, suggestions, onChange }: { value: string[]; suggestions: string[]; onChange: (v: string[]) => void }) {
  const [text, setText] = useState('');
  const add = () => {
    const v = text.trim().replace(/^@+/, '');
    if (v && !value.includes(v)) onChange([...value, v]);
    setText('');
  };
  return (
    <div className="psv-recipients">
      <div className="psv-chips">
        {value.map((r) => (
          <span key={r} className="psv-chip">
            @{r}
            <button aria-label={`Remove ${r}`} onClick={() => onChange(value.filter((x) => x !== r))}>
              <X size={12} />
            </button>
          </span>
        ))}
      </div>
      <div className="psv-recipient-add">
        <input
          className="sc-input"
          list="psv-channels"
          value={text}
          onChange={(e) => setText(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === 'Enter') {
              e.preventDefault();
              add();
            }
          }}
          placeholder="slack-devex or lead@example.com"
          aria-label="Add a recipient"
        />
        <datalist id="psv-channels">
          {suggestions.map((h) => (
            <option key={h} value={h} />
          ))}
        </datalist>
        <button className="btn btn-sm" onClick={add} disabled={!text.trim()}>
          Add
        </button>
      </div>
    </div>
  );
}
