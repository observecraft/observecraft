// In-browser surveys for mock mode (Developer Portal → Surveys). State lives in memory for the session:
// one closed survey with fictional responses from the mock directory's members. Validation, states,
// duplicate prevention and results follow the server (routes/portal_surveys.rs) through ../portalSurveys/logic.
import { aggregate, checkAnswers, checkSurvey, percent, surveyState, teamRates } from '../portalSurveys/logic';
import { SURVEY_TEMPLATES } from '../portalSurveys/templates';
import type { Survey, SurveyAnswers, SurveyForm, SurveyInput, SurveyQuestion, SurveyResponseInput, SurveyResponseRow, SurveyResults } from '../portalSurveys/types';
import { mockMe, mockMyTeams, mockPerson, mockTeamRoster, mockUserIds } from './identity';

const DAY = 86_400_000;
const MAX_SURVEYS = 200;

interface StoredResponse {
  id: number;
  /** Absent for anonymous surveys: only `respondent` (the duplicate guard) is kept. */
  userId: string | null;
  respondent: string;
  teams: string[];
  answers: SurveyAnswers;
  submittedAt: number;
}

interface StoredSurvey {
  input: SurveyInput;
  id: string;
  createdBy: string;
  createdAt: number;
  updatedAt: number;
  responses: StoredResponse[];
}

let store: StoredSurvey[] | null = null;
let nextResponseId = 1;

function fail(message: string): never {
  throw new Error(message);
}

/** Deterministic pseudo-random numbers for the seeded answers. */
function seeded(seed: number) {
  let s = seed;
  return () => {
    s = (s * 1664525 + 1013904223) % 4294967296;
    return s / 4294967296;
  };
}

const TEXTS = [
  'Builds on the storefront monorepo take too long; caching would help a lot.',
  'The order-service runbook was great during the last incident.',
  'Preview environments for catalog-service would save us a day per feature.',
  'Flaky checkout tests block merges every week.',
  'More examples in the service templates, please.',
  'Getting access to billing dashboards took three tickets.',
];

function seedAnswers(questions: SurveyQuestion[], r: () => number): SurveyAnswers {
  const out: SurveyAnswers = {};
  for (const q of questions) {
    if (!q.required && r() < 0.25) continue;
    if (q.kind === 'likert') out[q.id] = Math.min(5, Math.max(1, Math.round(2.4 + r() * 2.8)));
    else if (q.kind === 'nps') out[q.id] = Math.min(10, Math.round(4 + r() * 6.9));
    else if (q.kind === 'single') out[q.id] = q.options[Math.floor(r() * q.options.length)];
    else if (q.kind === 'multi') {
      const picked = q.options.filter(() => r() < 0.35);
      if (picked.length) out[q.id] = picked;
    } else if (r() < 0.5) out[q.id] = TEXTS[Math.floor(r() * TEXTS.length)];
  }
  return out;
}

function surveys(): StoredSurvey[] {
  if (store) return store;
  const now = Date.now();
  const t = SURVEY_TEMPLATES[0];
  const input = checkSurvey({
    title: 'Q3 Developer Experience Survey',
    description: t.description,
    template: t.key,
    questions: t.questions,
    audienceTeams: [],
    anonymous: true,
    status: 'closed',
    opensAt: now - 40 * DAY,
    closesAt: now - 12 * DAY,
    notify: ['slack-devex'],
  });
  const r = seeded(7);
  const roster = mockTeamRoster();
  const people = mockUserIds().filter((id) => id !== mockMe().id);
  const responses: StoredResponse[] = people
    .filter(() => r() < 0.7)
    .map((id) => ({
      id: nextResponseId++,
      userId: null,
      respondent: id,
      teams: roster.filter((team) => team.members.includes(id)).map((team) => team.handle),
      answers: seedAnswers(input.questions, r),
      submittedAt: now - 39 * DAY + Math.floor(r() * 26 * DAY),
    }))
    .sort((a, b) => b.submittedAt - a.submittedAt);
  store = [{ input, id: 'a3f1c2d4-5e6f-4a7b-8c9d-0e1f2a3b4c5d', createdBy: mockMe().name, createdAt: now - 42 * DAY, updatedAt: now - 12 * DAY, responses }];
  return store;
}

function find(id: string): StoredSurvey {
  return surveys().find((s) => s.id === id) ?? fail('Survey not found');
}

function audienceSize(input: SurveyInput): number {
  if (!input.audienceTeams.length) return mockUserIds().length;
  const members = new Set(mockTeamRoster().flatMap((t) => (input.audienceTeams.includes(t.handle) ? t.members : [])));
  return members.size;
}

function view(s: StoredSurvey): Survey {
  const me = mockMe().id;
  return {
    ...structuredClone(s.input),
    id: s.id,
    state: surveyState(s.input.status, s.input.opensAt, s.input.closesAt, Date.now()),
    createdBy: s.createdBy,
    createdAt: s.createdAt,
    updatedAt: s.updatedAt,
    responses: s.responses.length,
    audienceSize: audienceSize(s.input),
    responded: s.responses.some((r) => r.respondent === me),
  };
}

const shape = (qs: SurveyQuestion[]) => JSON.stringify(qs.map((q) => [q.id, q.kind, q.options]));

export async function listSurveys(): Promise<Survey[]> {
  return surveys()
    .slice()
    .sort((a, b) => b.createdAt - a.createdAt)
    .map(view);
}

export async function getSurvey(id: string): Promise<Survey> {
  return view(find(id));
}

export async function createSurvey(input: SurveyInput): Promise<Survey> {
  const checked = checkSurvey(input);
  if (surveys().length >= MAX_SURVEYS) fail(`An organization has at most ${MAX_SURVEYS} surveys`);
  const now = Date.now();
  const s: StoredSurvey = { input: checked, id: crypto.randomUUID(), createdBy: mockMe().name, createdAt: now, updatedAt: now, responses: [] };
  surveys().push(s);
  return view(s);
}

export async function updateSurvey(id: string, input: SurveyInput): Promise<Survey> {
  const s = find(id);
  const checked = checkSurvey(input);
  if (s.responses.length) {
    if (shape(s.input.questions) !== shape(checked.questions)) fail("Questions can't be added, removed or changed in kind or options once responses arrived; duplicate the survey instead");
    if (s.input.anonymous !== checked.anonymous) fail("Whether the survey is anonymous can't change once responses arrived");
  }
  s.input = checked;
  s.updatedAt = Date.now();
  return view(s);
}

export async function deleteSurvey(id: string): Promise<void> {
  const list = surveys();
  const i = list.findIndex((s) => s.id === id);
  if (i < 0) fail('Survey not found');
  list.splice(i, 1);
}

const inAudience = (s: StoredSurvey) => !s.input.audienceTeams.length || s.input.audienceTeams.some((t) => mockMyTeams().includes(t));

export async function getSurveyForm(id: string): Promise<SurveyForm> {
  const s = find(id);
  const v = view(s);
  return {
    id: s.id,
    title: v.title,
    description: v.description,
    questions: v.questions,
    anonymous: v.anonymous,
    state: v.state,
    closesAt: v.closesAt,
    inAudience: inAudience(s),
    responded: v.responded,
  };
}

export async function submitSurveyResponse(id: string, input: SurveyResponseInput): Promise<void> {
  const s = find(id);
  if (surveyState(s.input.status, s.input.opensAt, s.input.closesAt, Date.now()) !== 'open') fail("This survey isn't accepting responses");
  if (!inAudience(s)) fail('This survey is for other teams');
  const answers = checkAnswers(s.input.questions, input.answers as SurveyAnswers);
  const me = mockMe().id;
  if (s.responses.some((r) => r.respondent === me)) fail('You have already responded to this survey');
  s.responses.unshift({ id: nextResponseId++, userId: s.input.anonymous ? null : me, respondent: me, teams: mockMyTeams(), answers, submittedAt: Date.now() });
}

export async function getSurveyResults(id: string): Promise<SurveyResults> {
  const s = find(id);
  const size = audienceSize(s.input);
  const teams = mockTeamRoster()
    .filter((t) => !s.input.audienceTeams.length || s.input.audienceTeams.includes(t.handle))
    .sort((a, b) => a.name.localeCompare(b.name))
    .map((t): [string, string, number] => [t.handle, t.name, t.members.length]);
  return {
    surveyId: s.id,
    responses: s.responses.length,
    audienceSize: size,
    responseRate: percent(Math.min(s.responses.length, size), size),
    questions: aggregate(
      s.input.questions,
      s.responses.map((r) => r.answers),
    ),
    teams: teamRates(
      teams,
      s.responses.map((r) => r.teams),
    ),
  };
}

export async function listSurveyResponses(id: string): Promise<SurveyResponseRow[]> {
  const s = find(id);
  return s.responses.map((r) => {
    const person = r.userId ? mockPerson(r.userId) : undefined;
    return {
      id: r.id,
      submittedAt: r.submittedAt,
      respondent: s.input.anonymous || !person ? null : `${person.name} <${person.email}>`,
      teams: s.input.anonymous ? [] : r.teams,
      answers: structuredClone(r.answers),
    };
  });
}
