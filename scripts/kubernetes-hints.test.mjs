// The Kubernetes hints of Agent checks (web/src/api/integrations/kubernetesHints.ts): valid Autodiscovery annotations.
import assert from 'node:assert/strict';
import { createRequire } from 'node:module';
import test from 'node:test';
import { kubernetesHint } from '../web/src/api/integrations/kubernetesHints.ts';

const yaml = createRequire(new URL('../web/package.json', import.meta.url))('js-yaml');

for (const [check, endpoint] of [
  ['gitlab', 'openmetrics_endpoint'],
  ['gitlab_runner', 'prometheus_endpoint'],
]) {
  test(`${check}: every variant is a pod annotation running the check`, () => {
    const hint = kubernetesHint(check);
    assert.ok(hint && hint.variants.length > 0, check);
    for (const v of hint.variants) {
      const annotations = yaml.load(v.annotations);
      const [key, value] = Object.entries(annotations)[0];
      assert.match(key, /^ad\.datadoghq\.com\/[a-z0-9-]+\.checks$/, v.label);
      const config = JSON.parse(value)[check];
      assert.ok(config && config.instances.length === 1, v.label);
      assert.match(config.instances[0][endpoint], /^http:\/\/%%host%%:?\d*\//, v.label);
    }
  });
}

test('checks without a hint have none', () => assert.equal(kubernetesHint('redisdb'), undefined));
