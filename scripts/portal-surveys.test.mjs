import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import test from 'node:test';
import { registerHooks } from 'node:module';

// The web sources use extensionless relative imports (bundler resolution); resolve them to `.ts` here.
registerHooks({
  resolve(specifier, context, next) {
    try {
      return next(specifier, context);
    } catch (err) {
      if (specifier.startsWith('.')) return next(`${specifier}.ts`, context);
      throw err;
    }
  },
  load(url, context, next) {
    if (url.endsWith('.json')) return { format: 'module', source: `export default ${readFileSync(new URL(url), 'utf8')};`, shortCircuit: true };
    return next(url, context);
  },
});
const logic = await import('../web/src/api/portalSurveys/logic.ts');
const { SURVEY_TEMPLATES } = await import('../web/src/api/portalSurveys/templates.ts');
const mock = await import('../web/src/api/mock/portalSurveys.ts');

const q = (id, kind, required = false, options = []) => ({ id, kind, title: `Question ${id}`, description: '', required, options });
const base = (questions) => ({
  title: ' Developer Experience Survey ',
  description: '',
  template: '',
  questions,
  audienceTeams: [],
  anonymous: false,
  status: 'draft',
  opensAt: null,
  closesAt: null,
  notify: [],
});

test('surveys are checked like the server', () => {
  const ok = logic.checkSurvey(base([q('tools', 'likert', true, ['ignored']), q('lang', 'single', false, [' Rust ', 'Go'])]));
  assert.equal(ok.title, 'Developer Experience Survey');
  assert.deepEqual(ok.questions[0].options, []);
  assert.deepEqual(ok.questions[1].options, ['Rust', 'Go']);
  assert.throws(() => logic.checkSurvey(base([])), /at least one question/);
  assert.throws(() => logic.checkSurvey(base([q('a', 'text'), q('a', 'nps')])), /twice/);
  assert.throws(() => logic.checkSurvey(base([q('Bad Id', 'text')])), /id/);
  assert.throws(() => logic.checkSurvey(base([q('a', 'single', false, ['only'])])), /2–20 options/);
  assert.throws(() => logic.checkSurvey({ ...base([q('a', 'text')]), opensAt: 2000, closesAt: 1000 }), /close after/);
  assert.throws(() => logic.checkSurvey({ ...base([q('a', 'text')]), notify: ['#general'] }), /#general/);
  assert.deepEqual(logic.checkSurvey({ ...base([q('a', 'text')]), notify: ['@slack-devex', 'lead@example.com'] }).notify, ['slack-devex', 'lead@example.com']);
});

test('answers are checked against the questions', () => {
  const qs = [q('tools', 'likert', true), q('nps', 'nps'), q('lang', 'single', false, ['Rust', 'Go']), q('areas', 'multi', false, ['CI', 'Docs']), q('more', 'text')];
  assert.deepEqual(logic.checkAnswers(qs, { tools: 4, nps: 10, lang: 'Go', areas: ['CI'], more: '  Faster builds ' }), { tools: 4, nps: 10, lang: 'Go', areas: ['CI'], more: 'Faster builds' });
  assert.deepEqual(logic.checkAnswers(qs, { tools: 1, more: ' ', areas: [], nps: null }), { tools: 1 });
  assert.throws(() => logic.checkAnswers(qs, {}), /Question tools/);
  assert.throws(() => logic.checkAnswers(qs, { tools: 6 }), /1 to 5/);
  assert.throws(() => logic.checkAnswers(qs, { tools: 3, nps: 11 }), /0 to 10/);
  assert.throws(() => logic.checkAnswers(qs, { tools: 3, lang: 'Java' }), /Java/);
  assert.throws(() => logic.checkAnswers(qs, { tools: 3, other: 1 }), /Unknown question other/);
});

test('results aggregate distributions, averages and the NPS score', () => {
  const qs = [q('tools', 'likert', true), q('nps', 'nps'), q('areas', 'multi', false, ['CI', 'Docs']), q('more', 'text')];
  const out = logic.aggregate(qs, [
    { tools: 5, nps: 10, areas: ['CI', 'Docs'], more: 'newest' },
    { tools: 4, nps: 9, areas: ['CI'] },
    { tools: 2, nps: 7 },
    { tools: 1, nps: 3, more: 'oldest' },
  ]);
  assert.deepEqual(
    out[0].counts.map((c) => c.count),
    [1, 1, 0, 1, 1],
  );
  assert.equal(out[0].average, 3);
  assert.deepEqual(out[1].nps, { score: 25, promoters: 2, passives: 1, detractors: 1 });
  assert.deepEqual(out[2].counts, [
    { label: 'CI', count: 2 },
    { label: 'Docs', count: 1 },
  ]);
  assert.deepEqual(out[3].texts, ['newest', 'oldest']);
  assert.equal(logic.aggregate(qs, [])[1].nps, null);
  assert.equal(logic.npsCategory(9), 'promoter');
  assert.equal(logic.npsCategory(7), 'passive');
  assert.equal(logic.npsCategory(6), 'detractor');
});

test('response rates per team', () => {
  const rates = logic.teamRates(
    [
      ['checkout', 'Checkout', 4],
      ['platform', 'Platform', 0],
    ],
    [['checkout'], ['checkout', 'platform'], []],
  );
  assert.deepEqual(rates[0], { team: 'checkout', name: 'Checkout', members: 4, responded: 2, rate: 50 });
  assert.equal(rates[1].rate, null);
});

test('states follow status and dates', () => {
  assert.equal(logic.surveyState('draft', null, null, 100), 'draft');
  assert.equal(logic.surveyState('open', 200, null, 100), 'scheduled');
  assert.equal(logic.surveyState('open', 50, 100, 100), 'closed');
  assert.equal(logic.surveyState('open', null, null, 100), 'open');
});

test('the CSV export quotes cells and defuses formulas', () => {
  const qs = [q('tools', 'likert'), q('areas', 'multi', false, ['CI', 'Docs']), q('more', 'text')];
  const csv = logic.responsesCsv(qs, [
    { id: 2, submittedAt: Date.UTC(2026, 9, 1, 12), respondent: 'Dana Lee <dana@example.com>', teams: ['checkout'], answers: { tools: 4, areas: ['CI', 'Docs'], more: 'Said "faster", please' } },
    { id: 1, submittedAt: Date.UTC(2026, 9, 1, 11), respondent: null, teams: [], answers: { more: '=HYPERLINK("x")' } },
  ]);
  const lines = csv.trim().split('\r\n');
  assert.equal(lines[0], 'Submitted at,Respondent,Teams,Question tools,Question areas,Question more');
  assert.equal(lines[1], '2026-10-01T12:00:00.000Z,Dana Lee <dana@example.com>,checkout,4 (Agree),CI; Docs,"Said ""faster"", please"');
  assert.equal(lines[2], `2026-10-01T11:00:00.000Z,Anonymous,,,,"'=HYPERLINK(""x"")"`);
});

test('every template is a valid survey', () => {
  assert.ok(SURVEY_TEMPLATES.length >= 3);
  for (const t of SURVEY_TEMPLATES) {
    const s = logic.checkSurvey({ ...base(t.questions), title: t.title, description: t.description, template: t.key });
    assert.ok(s.questions.length >= 4, t.key);
  }
});

test('the mock follows the respond flow', async () => {
  const created = await mock.createSurvey({ ...base(SURVEY_TEMPLATES[0].questions), title: 'Mock pulse', status: 'open' });
  assert.equal(created.state, 'open');
  const form = await mock.getSurveyForm(created.id);
  assert.equal(form.responded, false);
  const answers = Object.fromEntries(form.questions.filter((x) => x.required).map((x) => [x.id, x.kind === 'nps' ? 9 : x.kind === 'likert' ? 4 : x.options[0]]));
  await mock.submitSurveyResponse(created.id, { answers });
  await assert.rejects(mock.submitSurveyResponse(created.id, { answers }), /already responded/);
  const results = await mock.getSurveyResults(created.id);
  assert.equal(results.responses, 1);
  assert.equal((await mock.getSurvey(created.id)).responded, true);
  await assert.rejects(mock.updateSurvey(created.id, { ...base(SURVEY_TEMPLATES[0].questions.slice(1)), title: 'Mock pulse', status: 'open' }), /responses arrived/);
  await mock.deleteSurvey(created.id);
  await assert.rejects(mock.getSurvey(created.id), /not found/);
});
