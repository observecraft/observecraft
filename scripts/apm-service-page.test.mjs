import assert from 'node:assert/strict';
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
});
const logic = await import('../web/src/pages/apm/service/logic.ts');
const apm = await import('../web/src/api/mock/apm.ts');

const now = Date.now();
const lastHour = { from: now - 3600e3, to: now };

test("the env selector shows each env's requests per second, and NO DATA without any", () => {
  const envs = [
    { env: 'dev', reqPerSec: 0.4 },
    { env: 'it', reqPerSec: null },
    { env: 'prod', reqPerSec: 0.2 },
  ];
  assert.deepEqual(
    logic.envOptions(envs, null).map((o) => [o.value, o.label, o.hint]),
    [
      ['*', '*', '0.6'],
      ['dev', 'dev', '0.4'],
      ['it', 'it', 'NO DATA'],
      ['prod', 'prod', '0.2'],
    ],
  );
  // The env in the URL stays selectable when the service has none of its requests.
  assert.deepEqual(
    logic.envOptions(envs, 'qa').map((o) => [o.value, o.hint]),
    [
      ['*', '0.6'],
      ['dev', '0.4'],
      ['it', 'NO DATA'],
      ['prod', '0.2'],
      ['qa', 'NO DATA'],
    ],
  );
  assert.deepEqual([0, 0.004, 1.234, 12.34, 1234].map(logic.envRateLabel), ['0', '<0.01', '1.23', '12.3', '1234']);
});

test("the mock's service filters list env rates and an env without traffic", () => {
  const f = apm.getServiceFilters({ range: lastHour, service: 'storefront-api' });
  const byEnv = Object.fromEntries(f.envs.map((e) => [e.env, e.reqPerSec]));
  assert.ok(byEnv.prod > 0 && byEnv.staging > 0, JSON.stringify(f.envs));
  assert.ok(byEnv.prod > byEnv.staging);
  assert.equal(byEnv.dev, null, 'dev had requests before the range only');
  const summary = apm.getServiceSummary({ range: lastHour, service: 'storefront-api' });
  assert.ok(Math.abs(byEnv.prod + byEnv.staging - summary.reqPerSec) < 1e-9, 'the envs add up to the service');
});
