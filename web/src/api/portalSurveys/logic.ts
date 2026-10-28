// Surveys: validation, states and result aggregation shared by the pages and the in-browser mock, plus
// the CSV export. They follow the server (routes/portal_surveys.rs).
import type { SurveyAnswers, SurveyInput, SurveyQuestion, SurveyQuestionKind, SurveyQuestionResult, SurveyResponseRow, SurveyState, SurveyStatus, SurveyTeamRate } from './types';

export const LIKERT_LABELS = ['Strongly disagree', 'Disagree', 'Neutral', 'Agree', 'Strongly agree'];
export const NPS_SCALE = Array.from({ length: 11 }, (_, i) => i);

export const KIND_LABEL: Record<SurveyQuestionKind, string> = {
  likert: 'Agreement scale (1–5)',
  single: 'Single choice',
  multi: 'Multiple choice',
  text: 'Free text',
  nps: 'Net Promoter Score (0–10)',
};

export const STATE_LABEL: Record<SurveyState, string> = { draft: 'Draft', scheduled: 'Scheduled', open: 'Open', closed: 'Closed' };

const MAX_TITLE = 200;
const MAX_DESCRIPTION = 2000;
const MAX_QUESTIONS = 50;
const MAX_QUESTION = 300;
const MAX_OPTION = 200;
const MAX_TEXT_ANSWER = 5000;
const MAX_TEXTS = 500;

function text(value: string | undefined, what: string, max: number, required: boolean): string {
  const v = (value ?? '').trim();
  if (required && !v) throw new Error(`${what} is required`);
  if ([...v].length > max) throw new Error(`${what} is longer than ${max} characters`);
  return v;
}

const validHandle = (h: string) => h.length > 0 && h.length <= 100 && /^[A-Za-z0-9._-]+$/.test(h);

function validRecipient(r: string): boolean {
  const at = r.indexOf('@');
  if (at >= 0) return at > 0 && r.slice(at + 1).includes('.') && !/\s/.test(r) && r.length <= 254;
  return (r.startsWith('slack-') || r.startsWith('webhook-')) && validHandle(r);
}

/** Checks and normalises a survey; throws with the server's message. */
export function checkSurvey(input: SurveyInput): SurveyInput {
  const title = text(input.title, 'The title', MAX_TITLE, true);
  const description = text(input.description, 'The description', MAX_DESCRIPTION, false);
  if (!input.questions.length) throw new Error('Add at least one question');
  if (input.questions.length > MAX_QUESTIONS) throw new Error(`A survey has at most ${MAX_QUESTIONS} questions`);
  const ids = new Set<string>();
  const questions = input.questions.map((q, i): SurveyQuestion => {
    const n = i + 1;
    const id = q.id.trim();
    if (!id || id.length > 40 || !/^[a-z0-9_-]+$/.test(id)) throw new Error(`Question ${n} needs an id of up to 40 characters a–z, 0–9, _ or -`);
    if (ids.has(id)) throw new Error(`The question id ${id} is used twice`);
    ids.add(id);
    const qTitle = text(q.title, `The title of question ${n}`, MAX_QUESTION, true);
    const qDescription = text(q.description, `The description of question ${n}`, MAX_DESCRIPTION, false);
    let options: string[] = [];
    if (q.kind === 'single' || q.kind === 'multi') {
      options = (q.options ?? []).map((o) => o.trim());
      if (options.length < 2 || options.length > 20 || options.some((o) => !o || [...o].length > MAX_OPTION)) throw new Error(`Question ${n} needs 2–20 options of up to ${MAX_OPTION} characters`);
      const dup = options.find((o, j) => options.indexOf(o) !== j);
      if (dup !== undefined) throw new Error(`Question ${n} lists the option ${dup} twice`);
    }
    return { id, kind: q.kind, title: qTitle, description: qDescription, required: !!q.required, options };
  });
  const audienceTeams: string[] = [];
  for (const raw of input.audienceTeams) {
    const t = raw.trim();
    if (!validHandle(t)) throw new Error(`${t} isn't a team handle`);
    if (!audienceTeams.includes(t)) audienceTeams.push(t);
  }
  const notify: string[] = [];
  for (const raw of input.notify) {
    const r = raw.trim().replace(/^@+/, '');
    if (!validRecipient(r)) throw new Error(`${r} isn't a member email or a channel handle (slack-…, webhook-…)`);
    if (!notify.includes(r)) notify.push(r);
  }
  if (input.opensAt != null && input.closesAt != null && input.closesAt <= input.opensAt) throw new Error('The survey must close after it opens');
  return { ...input, title, description, template: (input.template ?? '').trim(), questions, audienceTeams, notify };
}

const wholeNumber = (v: unknown): number | null => (typeof v === 'number' && Number.isInteger(v) ? v : null);

/** Checks a response; returns the answers to store (unanswered ones dropped). Throws with the server's message. */
export function checkAnswers(questions: SurveyQuestion[], answers: SurveyAnswers): SurveyAnswers {
  const unknown = Object.keys(answers).find((k) => !questions.some((q) => q.id === k));
  if (unknown) throw new Error(`Unknown question ${unknown}`);
  const out: SurveyAnswers = {};
  for (const q of questions) {
    const given = answers[q.id];
    let value: unknown = undefined;
    if (given !== undefined && given !== null) {
      if (q.kind === 'likert' || q.kind === 'nps') {
        const [lo, hi] = q.kind === 'likert' ? [1, 5] : [0, 10];
        const n = wholeNumber(given);
        if (n === null || n < lo || n > hi) throw new Error(`Answer “${q.title}” from ${lo} to ${hi}`);
        value = n;
      } else if (q.kind === 'single') {
        if (typeof given !== 'string') throw new Error(`Pick one option for “${q.title}”`);
        if (!q.options.includes(given)) throw new Error(`${given} isn't an option of “${q.title}”`);
        value = given;
      } else if (q.kind === 'multi') {
        if (!Array.isArray(given) || given.some((g) => typeof g !== 'string')) throw new Error(`Answer “${q.title}” with a list of options`);
        const picked: string[] = [];
        for (const g of given as string[]) {
          if (!q.options.includes(g)) throw new Error(`${g} isn't an option of “${q.title}”`);
          if (picked.includes(g)) throw new Error(`${g} is picked twice for “${q.title}”`);
          picked.push(g);
        }
        value = picked.length ? picked : undefined;
      } else {
        if (typeof given !== 'string') throw new Error(`Answer “${q.title}” with text`);
        const s = given.trim();
        if ([...s].length > MAX_TEXT_ANSWER) throw new Error(`The answer to “${q.title}” is longer than ${MAX_TEXT_ANSWER} characters`);
        value = s || undefined;
      }
    }
    if (value !== undefined) out[q.id] = value;
    else if (q.required) throw new Error(`Answer “${q.title}”`);
  }
  return out;
}

/** Whether a survey accepts responses at `now`. */
export function surveyState(status: SurveyStatus, opensAt: number | null | undefined, closesAt: number | null | undefined, now: number): SurveyState {
  if (status === 'draft') return 'draft';
  if (status === 'closed') return 'closed';
  if (closesAt != null && closesAt <= now) return 'closed';
  if (opensAt != null && opensAt > now) return 'scheduled';
  return 'open';
}

/** `part` of `whole` in percent, one decimal; null without a whole. */
export const percent = (part: number, whole: number): number | null => (whole > 0 ? Math.round((part * 1000) / whole) / 10 : null);

export const npsCategory = (n: number): 'promoter' | 'passive' | 'detractor' => (n >= 9 ? 'promoter' : n >= 7 ? 'passive' : 'detractor');

/** Per-question results, responses newest first. */
export function aggregate(questions: SurveyQuestion[], responses: SurveyAnswers[]): SurveyQuestionResult[] {
  return questions.map((q) => {
    const out: SurveyQuestionResult = { questionId: q.id, kind: q.kind, title: q.title, answered: 0, counts: [], average: null, nps: null, texts: [] };
    const given = responses.map((r) => r[q.id]).filter((v) => v !== undefined && v !== null);
    if (q.kind === 'likert' || q.kind === 'nps') {
      const likert = q.kind === 'likert';
      const [lo, hi] = likert ? [1, 5] : [0, 10];
      const counts = new Array(hi - lo + 1).fill(0) as number[];
      let sum = 0;
      for (const v of given) {
        const n = wholeNumber(v);
        if (n === null || n < lo || n > hi) continue;
        counts[n - lo] += 1;
        sum += n;
        out.answered += 1;
      }
      out.counts = counts.map((count, i) => ({ label: likert ? LIKERT_LABELS[i] : String(i), count }));
      if (out.answered) {
        if (likert) out.average = Math.round((sum / out.answered) * 100) / 100;
        else {
          const promoters = counts[9] + counts[10];
          const passives = counts[7] + counts[8];
          const detractors = counts.slice(0, 7).reduce((a, b) => a + b, 0);
          out.nps = { score: Math.round(((promoters - detractors) * 1000) / out.answered) / 10, promoters, passives, detractors };
        }
      }
    } else if (q.kind === 'single' || q.kind === 'multi') {
      const counts = new Map<string, number>();
      for (const v of given) {
        const picked = (typeof v === 'string' ? [v] : Array.isArray(v) ? v.filter((x): x is string => typeof x === 'string') : []).filter((p) => q.options.includes(p));
        if (picked.length) out.answered += 1;
        for (const p of picked) counts.set(p, (counts.get(p) ?? 0) + 1);
      }
      out.counts = q.options.map((label) => ({ label, count: counts.get(label) ?? 0 }));
    } else {
      for (const v of given) {
        if (typeof v !== 'string') continue;
        out.answered += 1;
        if (out.texts.length < MAX_TEXTS) out.texts.push(v);
      }
    }
    return out;
  });
}

/** Response rate per team from (handle, name, members) and each response's teams. */
export function teamRates(teams: [string, string, number][], responses: string[][]): SurveyTeamRate[] {
  return teams.map(([team, name, members]) => {
    const responded = responses.filter((r) => r.includes(team)).length;
    return { team, name, members, responded, rate: percent(Math.min(responded, members), members) };
  });
}

/** An answer as one CSV cell. */
function answerText(q: SurveyQuestion, v: unknown): string {
  if (v === undefined || v === null) return '';
  if (q.kind === 'likert' && typeof v === 'number') return `${v} (${LIKERT_LABELS[v - 1] ?? '?'})`;
  if (Array.isArray(v)) return v.join('; ');
  return String(v);
}

/** Quotes a cell when needed; cells that a spreadsheet would run as a formula are prefixed with '. */
function cell(raw: string): string {
  const v = /^[=+\-@\t\r]/.test(raw) ? `'${raw}` : raw;
  return /[",\r\n]/.test(v) ? `"${v.replace(/"/g, '""')}"` : v;
}

/** The responses as CSV (RFC 4180, CRLF), one column per question. */
export function responsesCsv(questions: SurveyQuestion[], rows: SurveyResponseRow[]): string {
  const header = ['Submitted at', 'Respondent', 'Teams', ...questions.map((q) => q.title)];
  const lines = [
    header,
    ...rows.map((r) => [new Date(r.submittedAt).toISOString(), r.respondent ?? 'Anonymous', r.teams.join('; '), ...questions.map((q) => answerText(q, (r.answers as SurveyAnswers)[q.id]))]),
  ];
  return lines.map((l) => l.map(cell).join(',')).join('\r\n') + '\r\n';
}

/** A question id not used yet (`q1`, `q2`, …). */
export function newQuestionId(questions: { id: string }[]): string {
  for (let i = questions.length + 1; ; i++) {
    const id = `q${i}`;
    if (!questions.some((q) => q.id === id)) return id;
  }
}

/** An empty question of a kind. */
export function blankQuestion(kind: SurveyQuestionKind, questions: { id: string }[]): SurveyQuestion {
  return { id: newQuestionId(questions), kind, title: '', description: '', required: false, options: kind === 'single' || kind === 'multi' ? ['Option 1', 'Option 2'] : [] };
}
