// Tiles that carry both an OpenTelemetry Collector template and a Datadog Agent check (PostgreSQL, Redis): the
// Configure tab leads with the Agent's steps, the Collector as the alternative.
import assert from 'node:assert/strict';
import test from 'node:test';
import { createRequire } from 'node:module';
import { fileURLToPath } from 'node:url';

const require = createRequire(new URL('../web/package.json', import.meta.url));
const { outputFiles } = require('esbuild').buildSync({
  stdin: { contents: "export { integrationMetadata } from './api/mock/integrations';", resolveDir: fileURLToPath(new URL('../web/src', import.meta.url)), loader: 'ts' },
  bundle: true,
  platform: 'node',
  format: 'esm',
  write: false,
  define: { 'import.meta.env': '{}', 'import.meta.glob': 'globalThis.__noGlob' },
  logLevel: 'error',
});
globalThis.__noGlob = () => ({});
const web = await import(`data:text/javascript;base64,${Buffer.from(outputFiles[0].text).toString('base64')}`);

for (const [id, check] of [
  ['postgresql', 'postgres'],
  ['redis', 'redisdb'],
]) {
  test(`${id}: the Agent's ${check} check first, the Collector's receiver as the alternative`, () => {
    const d = web.integrationMetadata(id);
    const steps = d.setupSteps.join('\n');
    assert.equal(d.agent.check, check);
    assert.match(d.setupSteps[0], /Datadog Agent/);
    assert.match(steps, new RegExp(`conf\\.d/${check}\\.d/conf\\.yaml`));
    assert.match(steps, /OpenTelemetry Collector/);
    assert.ok(steps.indexOf('OpenTelemetry Collector') > steps.indexOf('Restart the Agent'));
  });
}

test('PostgreSQL says Database Monitoring is not supported', () => assert.match(web.integrationMetadata('postgresql').setupSteps.join('\n'), /Database Monitoring .* not supported/));
