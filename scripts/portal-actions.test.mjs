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
const logic = await import('../web/src/api/portalActions/logic.ts');
const { BLUEPRINTS } = await import('../web/src/api/portalActions/blueprints.ts');
const mock = await import('../web/src/api/mock/portalActions.ts');

const SECRETS = { github_token: 'ghp_s3cr3t/t0ken' };
const ctx = (inputs, steps = []) => ({ inputs, userEmail: 'dana@example.com', userName: 'Dana Lee', secrets: SECRETS, steps });
const input = (key, kind = 'text', extra = {}) => ({ key, label: key, kind, required: false, options: [], default: '', description: '', ...extra });

test('URLs percent-encode every value, like the server', () => {
  const c = ctx({ owner: 'example-org', repo: 'web shop?x=1#frag', path: 'envs/prod/main.tf', n: 3 });
  assert.equal(logic.renderUrl('https://api.github.com/repos/{{inputs.owner}}/{{ inputs.repo }}/generate', c), 'https://api.github.com/repos/example-org/web%20shop%3Fx%3D1%23frag/generate');
  assert.equal(logic.renderUrl('https://h.example.com/c/{{inputs.path}}', c), 'https://h.example.com/c/envs%2Fprod%2Fmain.tf');
  assert.equal(logic.renderUrl('https://h.example.com/c/{{inputs.path | path}}?n={{inputs.n}}', c), 'https://h.example.com/c/envs/prod/main.tf?n=3');
  assert.equal(logic.renderUrl('https://h.example.com/u?e={{user.email}}', c), 'https://h.example.com/u?e=dana%40example.com');
  assert.throws(() => logic.renderUrl('https://h.example.com/r/{{inputs.r}}', ctx({ r: '..' })), /\.\./);
  assert.throws(() => logic.renderUrl('https://h.example.com/r/{{inputs.p | path}}', ctx({ p: 'a/../b' })), /\.\./);
});

test('JSON bodies keep their structure and lone placeholders keep their type', () => {
  const c = ctx({ name: 'shop", "admin": true, "x": "', private: true, count: 2, desc: 'line\nbreak' });
  const v = JSON.parse(
    logic.renderJson('{"name": "{{inputs.name}}", "private": "{{inputs.private}}", "n": "{{inputs.count}}", "about": "By {{user.name}}: {{inputs.desc}}", "tags": ["{{inputs.count}} repos"]}', c),
  );
  assert.equal(v.name, 'shop", "admin": true, "x": "');
  assert.equal(v.admin, undefined);
  assert.equal(v.private, true);
  assert.equal(v.n, 2);
  assert.equal(v.about, 'By Dana Lee: line\nbreak');
  assert.deepEqual(v.tags, ['2 repos']);
  assert.equal(JSON.parse(logic.renderJson('{"c": "{{inputs.desc | base64}}"}', c)).c, 'bGluZQpicmVhaw==');
  assert.equal(logic.renderJson('', c), '');
  const steps = ctx({ branch: 'b' }, [{ object: { sha: 'abc123' }, items: [{ id: 7 }] }]);
  assert.deepEqual(JSON.parse(logic.renderJson('{"sha": "{{steps.1.object.sha}}", "id": "{{steps.1.items.0.id}}"}', steps)), { sha: 'abc123', id: 7 });
  assert.throws(() => logic.renderText('{{steps.1.object.missing}}', steps, true), /steps\.1\.object\.missing/);
  assert.throws(() => logic.renderText('{{inputs.v}}', ctx({ v: 'a\r\nX: 1' }), true), /line break/);
});

test('templates only refer to known names; hosts are literal', () => {
  const scope = { inputs: [input('repo')], secrets: new Set(['github_token']), allowSecrets: true, priorSteps: 1 };
  logic.checkTemplate('{{inputs.repo}} {{user.email}} {{secrets.github_token}} {{steps.1.sha}} {{inputs.repo | path}}', scope);
  assert.throws(() => logic.checkTemplate('{{inputs.other}}', scope), /inputs\.other/);
  assert.throws(() => logic.checkTemplate('{{secrets.aws}}', scope), /secrets\.aws/);
  assert.throws(() => logic.checkTemplate('{{steps.2.sha}}', scope), /steps\.2/);
  assert.throws(() => logic.checkTemplate('{{inputs.repo | upper}}', scope), /upper/);
  assert.throws(() => logic.checkTemplate('{{secrets.github_token}}', { ...scope, allowSecrets: false }), /HTTP/);
  const req = (url, body = '') => ({ method: 'post', url, headers: [], body });
  assert.equal(logic.checkRequest(req(' https://api.github.com/x/{{inputs.repo}} '), scope).method, 'POST');
  assert.throws(() => logic.checkRequest(req('https://{{inputs.repo}}/x'), scope), /host/);
  assert.throws(() => logic.checkRequest(req('ftp://example.com'), scope), /http/);
  assert.throws(() => logic.checkRequest(req('https://example.com', '{"p": {{inputs.repo}}}'), scope), /JSON/);
});

test('form fields and run inputs are checked', () => {
  assert.throws(() => logic.checkInputs([input('env', 'select')]), /options/);
  assert.throws(() => logic.checkInputs([input('Bad-Key')]), /Bad-Key/);
  assert.throws(() => logic.checkInputs([input('a'), input('a', 'number')]), /twice/);
  assert.throws(() => logic.checkInputs([input('p', 'boolean', { default: 'yes' })]), /true or false/);
  const defs = logic.checkInputs([
    input('env', 'select', { options: ['staging', 'production'], default: 'staging' }),
    input('name', 'text', { required: true }),
    input('private', 'boolean', { default: 'true' }),
    input('replicas', 'number'),
  ]);
  assert.deepEqual(logic.checkRunInputs(defs, { name: ' checkout-web ', replicas: '3' }), { env: 'staging', name: 'checkout-web', private: true, replicas: 3 });
  assert.throws(() => logic.checkRunInputs(defs, {}), /name is required/);
  assert.throws(() => logic.checkRunInputs(defs, { name: 'x', env: 'qa' }), /qa/);
  assert.throws(() => logic.checkRunInputs(defs, { name: 'x', replicas: 'many' }), /number/);
  assert.throws(() => logic.checkRunInputs(defs, { name: 'x', extra: 1 }), /extra/);
});

test('secrets are redacted raw, percent-encoded and base64', () => {
  assert.equal(logic.redact('token=ghp_s3cr3t/t0ken', SECRETS), 'token=[REDACTED]');
  assert.equal(logic.redact('u=ghp_s3cr3t%2Ft0ken', SECRETS), 'u=[REDACTED]');
  assert.equal(logic.redact(`basic ${Buffer.from('ghp_s3cr3t/t0ken').toString('base64')}`, SECRETS), 'basic [REDACTED]');
  assert.deepEqual(logic.mergeSecrets({ a: '1', b: '2' }, { a: null, c: ' 3 ' }), { b: '2', c: '3' });
  assert.equal(logic.resultLink({ key: 'SHOP-12', self: 'https://example.atlassian.net/rest/api/3/issue/1' }), 'https://example.atlassian.net/browse/SHOP-12');
});

test('every blueprint passes the checks a saved action gets', () => {
  assert.equal(BLUEPRINTS.length, 8);
  for (const b of BLUEPRINTS) {
    const inputs = logic.checkInputs(b.inputs);
    const names = new Set(b.secrets.map((s) => s.name));
    logic.checkStep(b.step, inputs, names);
    assert.deepEqual(logic.secretsUsed(b.step), [...names].sort(), `${b.key} declares the secrets it uses`);
    assert.ok(b.description.length > 80, `${b.key} has a description`);
  }
});

test('mock: create from a blueprint, run, and never return secrets', async () => {
  mock.resetMock();
  const tf = BLUEPRINTS.find((b) => b.key === 'terraform_pr');
  const created = await mock.createAction({ name: 'Terraform PR', category: tf.category, blueprint: tf.key, inputs: tf.inputs, step: tf.step, secrets: { github_token: 'ghp_mockSecret999' } });
  assert.deepEqual(created.secretNames, ['github_token']);
  assert.ok(!JSON.stringify(created).includes('ghp_mockSecret999'));
  await assert.rejects(() => mock.createAction({ name: 'No secret', category: tf.category, inputs: tf.inputs, step: tf.step }), /github_token/);
  const run = await mock.runAction(created.id, { branch: 'add-bucket', content: 'resource "x" "y" {}', title: 'Add bucket' });
  assert.equal(run.status, 'succeeded');
  assert.match(run.link, /^https:\/\/github\.com\/example-org\/infrastructure\/pull\/128$/);
  await assert.rejects(() => mock.runAction(created.id, {}), /New branch is required/);
  const runs = await mock.listRuns(created.id);
  assert.equal(runs.length, 1);
  const list = await mock.listActions();
  assert.equal(list.find((a) => a.id === created.id).lastRunStatus, 'succeeded');
  const incident = BLUEPRINTS.find((b) => b.key === 'declare_incident');
  const inc = await mock.createAction({ name: 'Declare', category: incident.category, inputs: incident.inputs, step: incident.step });
  const incRun = await mock.runAction(inc.id, { title: 'Checkout latency' });
  assert.match(incRun.link, /^\/incidents\/\d+$/);
  const updated = await mock.updateAction(created.id, { name: 'Terraform PR', category: tf.category, inputs: tf.inputs, step: tf.step, secrets: {} });
  assert.deepEqual(updated.secretNames, ['github_token'], 'secrets are kept when not sent');
  await mock.deleteAction(created.id);
  await assert.rejects(() => mock.getAction(created.id), /not found/);
});
