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
const logic = await import('../web/src/api/workflows/logic.ts');
const layout = await import('../web/src/pages/automation/workflows/canvasLayout.ts');
const { variablesFor, loopsAround } = await import('../web/src/pages/automation/workflows/variables.ts');
const { BLUEPRINTS, matchBlueprints } = await import('../web/src/pages/automation/workflows/blueprints.ts');
const { starterStep } = await import('../web/src/pages/automation/workflows/catalog.ts');

const VARS = {
  Trigger: { monitor: { name: 'High error rate on order-service', tags: ['team:checkout', 'env:prod'] }, value: '7.5', transition: { to: 'alert' } },
  Steps: { check: { status: 'succeeded', output: { value: 3, items: [{ id: 'a1' }, { id: 'b2' }] } } },
  inputs: { env: 'production', dry_run: false, replicas: 0 },
};
const run = (src) => logic.evalExpr(logic.parseExpr(src), VARS);
const step = (id, kind = 'notify', extra = {}) => logic.normalizeStep({ id, kind, fields: kind === 'notify' ? { recipients: '@slack-checkout', message: 'Hello' } : {}, ...extra });
const spec = (steps, extra = {}) => ({ inputs: [], triggers: [logic.emptyTrigger('manual')], steps, ...extra });

test('expressions read paths and literals like the server', () => {
  assert.equal(run('Trigger.monitor.name'), 'High error rate on order-service');
  assert.equal(run('{{ Steps.check.output.items[1].id }}'), 'b2');
  assert.equal(run('Steps.check.output.items.0.id'), 'a1');
  assert.equal(run('Steps.missing.output'), null);
  assert.equal(run("'it\\'s'"), "it's");
  assert.equal(run('-2.5'), -2.5);
});

test('comparisons coerce numeric text', () => {
  assert.equal(run('Trigger.value > 5'), true);
  assert.equal(run('Trigger.value == 7.5'), true);
  assert.equal(run('Steps.check.output.value <= 3'), true);
  assert.equal(run('Steps.check.output.value != 3'), false);
  assert.equal(run('inputs.env == "production"'), true);
  assert.equal(run("inputs.env < 'staging'"), true);
  assert.equal(run('inputs.env > 3'), false, 'text and numbers do not order');
  assert.equal(run('Steps.missing == null'), true);
});

test('and, or, not and contains', () => {
  assert.equal(run("Trigger.transition.to == 'alert' && Trigger.value > 5"), true);
  assert.equal(run("Trigger.transition.to == 'ok' or inputs.env == 'production'"), true);
  assert.equal(run('not inputs.dry_run'), true);
  assert.equal(run('!inputs.replicas'), true);
  assert.equal(run("Trigger.monitor.tags contains 'team:checkout'"), true);
  assert.equal(run("Trigger.monitor.name contains 'order-service'"), true);
  assert.equal(run("(inputs.env == 'staging' || Steps.check.status == 'succeeded') && !(Trigger.value < 1)"), true);
});

test('expression mistakes are explained', () => {
  assert.match(logic.exprError(''), /empty/);
  assert.match(logic.exprError('a =='), /ends too early/);
  assert.match(logic.exprError('(Trigger.a == 1'), /parenthesis/);
  assert.match(logic.exprError('Trigger.a == 1 b'), /&&/);
  assert.match(logic.exprError('1 < Trigger.a < 3'), /chained/);
  assert.match(logic.exprError("'open"), /quote/);
  assert.match(logic.exprError('a; drop'), /Unexpected character/);
  assert.match(logic.exprError('Secrets.token == 1'), /Secrets/);
  assert.equal(logic.exprError("Steps.a.status == 'succeeded'"), null);
});

test('simulate: if/else picks a branch from earlier results', () => {
  const s = spec([
    step('check', 'metric_query', { fields: { query: 'q' } }),
    step('gate', 'if', { expression: 'Steps.check.output.value > 5', steps: [step('restart'), step('tell')], elseSteps: [step('note')] }),
    step('after'),
  ]);
  const perform = (value) => (st) => ({ ok: true, inputs: {}, output: st.id === 'check' ? { value } : {} });
  assert.deepEqual(
    logic.simulate(s, {}, perform(9)).steps.map((x) => x.stepId),
    ['check', 'gate', 'restart', 'tell', 'after'],
  );
  const low = logic.simulate(s, {}, perform(1));
  assert.deepEqual(
    low.steps.map((x) => x.stepId),
    ['check', 'gate', 'note', 'after'],
  );
  assert.deepEqual(low.steps[1].output, { result: false, branch: 'else' });
});

test('simulate: for each runs per item with Loop vars, and stops on failures', () => {
  const s = spec([
    step('each', 'for_each', { expression: 'Trigger.items', steps: [step('is_billing', 'if', { expression: "Loop.item == 'billing'", steps: [step('page')], elseSteps: [step('note')] })] }),
  ]);
  const seen = [];
  const r = logic.simulate(s, { Trigger: { items: ['order-service', 'billing'] } }, (st, vars) => {
    seen.push([st.id, vars.Loop.item, vars.Loop.index]);
    return { ok: true, inputs: {}, output: {} };
  });
  assert.equal(r.ok, true);
  assert.deepEqual(seen, [
    ['note', 'order-service', 0],
    ['page', 'billing', 1],
  ]);
  assert.equal(r.steps.find((x) => x.stepId === 'page').iteration, 1);
  const tooMany = logic.simulate(s, { Trigger: { items: Array.from({ length: 101 }, (_, i) => i) } }, () => ({ ok: true, inputs: {}, output: {} }));
  assert.match(tooMany.error, /100 items/);
  const notList = logic.simulate(s, { Trigger: { items: 'nope' } }, () => ({ ok: true, inputs: {}, output: {} }));
  assert.match(notList.error, /needs a list/);
  const failing = logic.simulate(spec([step('a'), step('b')]), {}, (st) => ({ ok: st.id !== 'a', inputs: {}, output: {}, error: 'boom' }));
  assert.equal(failing.ok, false);
  assert.match(failing.error, /Step a: boom/);
  const tolerant = logic.simulate(spec([step('a', 'notify', { continueOnError: true, fields: { recipients: 'x', message: 'y' } }), step('b')]), {}, (st) => ({
    ok: st.id !== 'a',
    inputs: {},
    output: {},
    error: 'boom',
  }));
  assert.equal(tolerant.ok, true);
});

test('step tree helpers', () => {
  let list = [step('a'), step('gate', 'if', { expression: 'true' })];
  list = logic.insertStep(list, { parent: 'gate', branch: 'elseSteps' }, 0, step('b'));
  list = logic.insertStep(list, { parent: null, branch: 'steps' }, 1, step('c'));
  assert.deepEqual(
    list.map((s) => s.id),
    ['a', 'c', 'gate'],
  );
  assert.equal(logic.findStep(list, 'b').id, 'b');
  assert.equal(logic.freeId(list, 'a'), 'a_2');
  assert.equal(logic.freeId(list, 'New Step!'), 'new_step');
  list = logic.replaceStep(list, 'b', { ...step('b'), name: 'Renamed' });
  assert.equal(logic.findStep(list, 'b').name, 'Renamed');
  list = logic.removeStep(list, 'b');
  assert.equal(logic.findStep(list, 'b'), undefined);
  assert.equal(logic.allSteps(list).length, 3);
});

test('spec checks follow the server', () => {
  assert.equal(logic.specError(spec([step('a')])), null);
  assert.match(logic.specError(spec([step('a'), step('a')])), /twice/);
  assert.match(logic.specError(spec([step('Bad Id')])), /Bad Id/);
  assert.match(logic.specError(spec([step('n', 'notify', { fields: { recipients: '@x' } })])), /message/);
  assert.match(logic.specError(spec([step('n', 'notify', { fields: { recipients: '@x', message: '{{ inputs.nope }}' } })])), /nope/);
  const three = step('l1', 'for_each', {
    expression: 'Trigger.a',
    steps: [step('l2', 'for_each', { expression: 'Trigger.a', steps: [step('l3', 'for_each', { expression: 'Trigger.a', steps: [step('x')] })] })],
  });
  assert.match(logic.specError(spec([three])), /nested 2/);
});

test('describeCron', () => {
  assert.equal(logic.describeCron('0 9 * * 1-5', 'Europe/Paris'), 'At 09:00, Monday through Friday (Europe/Paris)');
  assert.equal(logic.describeCron('30 7 * * *', 'UTC'), 'At 07:30, every day UTC');
  assert.equal(logic.describeCron('*/15 * * * *'), 'Every 15 minutes');
  assert.equal(logic.describeCron('0 * * * *'), 'Every hour');
  assert.equal(logic.describeCron('0 0 1 * *', 'UTC'), '0 0 1 * * UTC');
});

test('canvas layout: a vertical flow with insertion points', () => {
  const l = layout.layoutWorkflow([step('a'), step('b')]);
  const [trigger, a, b] = l.nodes;
  assert.equal(trigger.id, layout.TRIGGER_ID);
  assert.equal(a.x, trigger.x, 'one column');
  assert.equal(a.y, layout.NODE_H + layout.GAP_Y);
  assert.equal(b.y, a.y + layout.NODE_H + layout.GAP_Y);
  // A "+" before each step and one after the last.
  assert.deepEqual(
    l.adds.map((x) => x.index),
    [0, 1, 2],
  );
  assert.ok(l.height > b.y + layout.NODE_H);
  assert.equal(l.width, layout.NODE_W);
});

test('canvas layout: if/else branches side by side, for each framed', () => {
  const gate = step('gate', 'if', { expression: 'true', steps: [step('t1'), step('t2')], elseSteps: [] });
  const l = layout.layoutWorkflow([gate, step('after')]);
  const at = (id) => l.nodes.find((n) => n.id === id);
  assert.ok(at('t1').x < at('gate').x, 'true branch on the left');
  assert.equal(l.width, layout.NODE_W + layout.GAP_X + layout.EMPTY_W);
  assert.ok(at('after').y > at('t2').y + layout.NODE_H, 'the flow merges below the longest branch');
  assert.equal(at('after').x, at('gate').x);
  assert.deepEqual(
    l.labels.map((x) => x.text),
    ['true', 'false'],
  );
  const branchAdds = l.adds.filter((x) => x.ref.parent === 'gate');
  assert.deepEqual(
    branchAdds.map((x) => `${x.ref.branch}:${x.index}`),
    ['steps:0', 'steps:1', 'steps:2', 'elseSteps:0'],
  );
  const loop = layout.layoutWorkflow([step('each', 'for_each', { expression: 'Trigger.items', steps: [step('inner')] })]);
  const frame = loop.frames[0];
  const inner = loop.nodes.find((n) => n.id === 'inner');
  assert.equal(frame.id, 'each');
  assert.ok(frame.x < inner.x && frame.x + frame.w > inner.x + layout.NODE_W, 'the frame holds the body');
  assert.ok(frame.y + frame.h > inner.y + layout.NODE_H);
});

test('variables follow the triggers, earlier steps and loops', () => {
  const s = {
    inputs: [{ key: 'env', label: 'Environment', kind: 'text', required: false, options: [], default: '', description: '' }],
    triggers: [logic.emptyTrigger('monitor')],
    steps: [step('check', 'metric_query', { fields: { query: 'q' } }), step('each', 'for_each', { expression: 'Trigger.items', steps: [step('inner')] })],
  };
  const paths = (id) => variablesFor(s, id).map((v) => v.path);
  assert.ok(paths('inner').includes('Trigger.monitor.name'));
  assert.ok(paths('inner').includes('inputs.env'));
  assert.ok(paths('inner').includes('Steps.check.output.value'));
  assert.ok(paths('inner').includes('Loop.item'));
  assert.ok(!paths('check').includes('Loop.item'));
  assert.ok(!paths('check').includes('Steps.check.status'), 'not its own result');
  assert.deepEqual(
    loopsAround(s.steps, 'inner').map((x) => x.id),
    ['each'],
  );
});

test('blueprints and starter steps pass the checks', () => {
  for (const b of BLUEPRINTS) assert.equal(logic.specError(b.build().spec), null, b.key);
  for (const kind of Object.keys(logic.STEP_LABEL)) assert.equal(logic.specError(spec([starterStep('s', kind)])), null, kind);
  assert.equal(matchBlueprints('send a daily report of the top errors')[0].key, 'daily_error_report');
  assert.equal(matchBlueprints('page on-call when the SLO burns')[0].key, 'page_on_slo_burn');
  assert.deepEqual(matchBlueprints('   '), []);
});
