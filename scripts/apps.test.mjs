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
const logic = await import('../web/src/pages/automation/apps/appLogic.ts');
const { APP_BLUEPRINTS } = await import('../web/src/pages/automation/apps/blueprints.ts');
const mock = await import('../web/src/api/mock/apps.ts');

const c = (id, kind, extra = {}) => ({ ...logic.blankComponent(id, kind), ...extra });
const restartApp = () => ({
  components: [
    c('env', 'select', { options: ['staging', 'production'], defaultValue: 'production', width: 4 }),
    c('replicas', 'number_input', { defaultValue: '3', width: 4 }),
    c('restart', 'button', {
      workflowId: 'w1',
      inputs: { environment: '{{ components.env.value }}', replicas: '{{components.replicas.value}}', note: 'Restart in {{ components.env.value }}' },
      width: 4,
    }),
    c('results', 'table', { source: 'restart', path: 'Steps.restart.output.body' }),
  ],
});

test('apps are checked like the server does', () => {
  assert.equal(logic.specProblem(restartApp()), null);
  const missing = restartApp();
  missing.components[2].inputs.x = '{{ components.missing.value }}';
  assert.match(logic.specProblem(missing), /missing/);
  const secret = restartApp();
  secret.components[2].inputs.x = '{{ secrets.token }}';
  assert.match(logic.specProblem(secret), /components/);
  const source = restartApp();
  source.components[3].source = 'env';
  assert.match(logic.specProblem(source), /buttons/);
  const options = restartApp();
  options.components[0].options = [];
  assert.match(logic.specProblem(options), /options/);
  const twice = restartApp();
  twice.components[1].id = 'env';
  assert.match(logic.specProblem(twice), /twice/);
  const width = restartApp();
  width.components[1].width = 5;
  assert.match(logic.specProblem(width), /width/);
  const workflow = restartApp();
  workflow.components[2].workflowId = '';
  assert.match(logic.specProblem(workflow), /workflow/);
  assert.match(logic.specProblem({ components: [c('Bad Id', 'text', { text: 'x' })] }), /Bad Id/);
});

test('new components pass the checks and get free ids', () => {
  let spec = { components: [] };
  for (const kind of ['heading', 'text', 'text_input', 'number_input', 'select', 'button', 'table', 'metric']) {
    const next = logic.newComponent(spec, kind);
    spec = { components: [...spec.components, next] };
  }
  // Only the button lacks its workflow.
  assert.match(logic.specProblem(spec), /Component button: Choose the workflow/);
  assert.equal(spec.components.find((x) => x.kind === 'table').source, 'button');
  assert.equal(logic.newComponent(spec, 'button').id, 'button_2');
  assert.equal(logic.freeId({ components: [] }, 'Run It!'), 'run_it');
});

test('components pack into rows of 12 columns', () => {
  const rows = logic.layoutRows([
    c('a', 'text', { width: 4 }),
    c('b', 'text', { width: 4 }),
    c('c', 'text', { width: 6 }),
    c('d', 'text', { width: 6 }),
    c('e', 'text', { width: 12 }),
    c('f', 'text', { width: 3 }),
  ]);
  assert.deepEqual(
    rows.map((r) => r.map((x) => x.id)),
    [['a', 'b'], ['c', 'd'], ['e'], ['f']],
  );
  assert.deepEqual(logic.layoutRows([]), []);
});

test('defaults and button inputs', () => {
  const spec = restartApp();
  const values = logic.initialValues(spec);
  assert.deepEqual(values, { env: 'production', replicas: 3 });
  const expected = { environment: 'production', replicas: 3, note: 'Restart in production' };
  assert.deepEqual(logic.buttonInputsPreview(spec.components[2], values), expected);
  assert.deepEqual(mock.buttonInputs(spec.components[2], values), expected, 'the mock fills them the same way');
  assert.deepEqual(mock.buttonInputs(spec.components[2], {}), { note: 'Restart in' }, 'empty values are left out');
});

test('tables read the latest step results of a run', () => {
  const run = {
    trigger: { kind: 'app' },
    inputs: { service: 'order-service' },
    steps: [
      { stepId: 'pods', status: 'succeeded', output: { groups: [{ group: 'old', value: 1 }] } },
      { stepId: 'pods', status: 'succeeded', output: { groups: [{ group: 'kube_namespace:storefront', value: 4, extra: { a: 1 } }] } },
      { stepId: 'scale', status: 'succeeded', output: { body: { status: 'accepted' } } },
    ],
  };
  assert.deepEqual(logic.tableRows(run, 'Steps.pods.output.groups', ['group', 'value']), { columns: ['group', 'value'], rows: [{ group: 'kube_namespace:storefront', value: 4, extra: { a: 1 } }] });
  assert.deepEqual(logic.tableRows(run, 'Steps.pods.output.groups', []).columns, ['group', 'value', 'extra']);
  assert.deepEqual(logic.tableRows(run, 'Steps.scale.output.body', []).rows, [{ status: 'accepted' }], 'an object is one row');
  assert.deepEqual(logic.tableRows(run, 'inputs.service', []).rows, [{ value: 'order-service' }]);
  assert.deepEqual(logic.tableRows(run, 'Steps.none.output', []).rows, []);
  assert.deepEqual(logic.tableRows(undefined, 'Steps.pods.output.groups', ['a']).rows, []);
  assert.equal(logic.cellText({ a: 1 }), '{"a":1}');
  assert.equal(logic.cellText(null), '');
});

test('path suggestions, moving and renaming', () => {
  const spec = {
    inputs: [],
    triggers: [],
    steps: [{ id: 'gate', kind: 'if', steps: [{ id: 'pods', kind: 'metric_query', steps: [], elseSteps: [] }], elseSteps: [{ id: 'notify_ops', kind: 'notify', steps: [], elseSteps: [] }] }],
  };
  assert.deepEqual(logic.pathSuggestions(spec), ['Steps.pods.output.groups', 'Steps.pods.output.value', 'Steps.notify_ops.output.sent', 'Steps.notify_ops.output.failed']);
  assert.deepEqual(logic.pathSuggestions(null), []);
  const app = restartApp();
  assert.deepEqual(
    logic.moveComponent(app, 'replicas', -1).components.map((x) => x.id),
    ['replicas', 'env', 'restart', 'results'],
  );
  assert.equal(logic.moveComponent(app, 'env', -1), app, 'the first stays first');
  const renamed = logic.renameComponent(logic.renameComponent(app, 'env', 'environment'), 'restart', 'go');
  assert.equal(renamed.components[2].inputs.environment, '{{ components.environment.value }}');
  assert.equal(renamed.components[2].inputs.note, 'Restart in {{ components.environment.value }}');
  assert.equal(renamed.components[3].source, 'go');
});

test('blueprints are valid apps once wired to a workflow', () => {
  assert.ok(APP_BLUEPRINTS.length >= 3);
  for (const b of APP_BLUEPRINTS) {
    assert.equal(logic.specProblem(b.spec('7a3e1c20-5b6d-4e7f-9a1b-2c3d4e5f6a04')), null, b.id);
    if (b.needsWorkflow) assert.match(logic.specProblem(b.spec('')), /workflow/, b.id);
  }
});

test('mock apps run their buttons through the workflows mock', async () => {
  const apps = await mock.listApps();
  const ops = apps.find((a) => a.name === 'Service operations');
  assert.equal(logic.specProblem(ops.spec), null);
  const run = await mock.runApp(ops.id, 'scale', { service: 'catalog-service', environment: 'staging', replicas: 2 });
  assert.ok(run.runId > 0);
  await assert.rejects(async () => mock.runApp(ops.id, 'nope', {}), /no button nope/);
  await assert.rejects(async () => mock.createApp({ name: 'Service operations', spec: { components: [] } }), /exists already/);
});
