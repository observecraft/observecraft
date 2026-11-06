// Mock mode (make web-mock) charts data on every query of the demo accounts' cloud service dashboards
// (web/src/api/integrations/cloudServiceDashboards.generated.json), template variables at their default.
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
});
const engine = await import('../web/src/api/mock/metricsEngine.ts');
const mock = JSON.parse(readFileSync(new URL('../web/src/api/integrations/cloudServiceDashboards.generated.json', import.meta.url), 'utf8'));

/** A dashboard's metric queries, template variables (`$region`) at their default `*`. */
const queries = (d) => [...JSON.stringify(d.widgets).matchAll(/"query":"([^"]+)"/g)].map(([, q]) => q.replace(/\$[a-z_]+,?/g, '').replace(/,\}/, '}').replace(/\{\}/, '{*}'));

test('every chart of the CloudFront, ECS, Container Insights, RDS and WAF dashboards has data in mock mode', () => {
  const range = { from: Date.now() - 3_600_000, to: Date.now() };
  for (const logo of ['tile:aws-cloudfront', 'tile:aws-ecs', 'tile:aws-ecs-containerinsights', 'tile:aws-rds', 'tile:aws-wafv2']) {
    const d = mock.dashboards.find((x) => x.logo === logo);
    assert.ok(d, `${logo} is in the mock`);
    for (const q of queries(d)) {
      const groups = engine.evaluateGroups(q, range, 20);
      assert.ok(groups.length > 0, `${logo}: ${q} has no series`);
      assert.ok(
        groups.some((g) => g.points.some((p) => p[1] !== null && p[1] > 0)),
        `${logo}: ${q} is all zero`,
      );
    }
  }
});
