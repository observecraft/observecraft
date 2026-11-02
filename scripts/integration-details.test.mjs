// The integrations' details imported from integrations-core (scripts/import-integration-details.mjs).
import assert from 'node:assert/strict';
import { existsSync, readFileSync } from 'node:fs';
import test from 'node:test';
import { PINNED_REVISION } from './import-integration-dashboards.mjs';
import { changelog, htmlToMarkdown, logFacets, metadataCsv, monitorTemplate, readmeSections, resolveShortcodes } from './import-integration-details.mjs';

const README = `# Agent Check: Thing

## Overview

Thing does [things][1].

## Setup

### Installation

Install [the Agent][2].

<!-- xxx tabs xxx -->
<!-- xxx tab "Host" xxx -->
On a host.
<!-- xxz tab xxx -->
<!-- xxz tabs xxx -->

## Data Collected

### Metrics

See [metadata.csv][3].

### Service Checks

See [service_checks.json][4].

## Troubleshooting

Need help? Contact [Datadog support][5].

## Further Reading

- [Blog][6]

[1]: https://example.com/things
[2]: https://docs.example.com/agent
[3]: https://github.com/x/metadata.csv
[4]: https://github.com/x/service_checks.json
[5]: https://docs.example.com/help
[6]: https://example.com/blog
`;

test('README sections keep the reference links they use', () => {
  const s = readmeSections(README);
  assert.equal(s.overview, 'Thing does [things][1].\n\n[1]: https://example.com/things');
  assert.match(s.setup, /^### Installation\n\nInstall \[the Agent\]\[2\]\./);
  assert.match(s.setup, /<!-- xxx tab "Host" xxx -->/, 'tab markers stay for the UI');
  assert.match(s.setup, /\[2\]: https:\/\/docs\.example\.com\/agent$/);
  assert.doesNotMatch(s.setup, /\[1\]:/, 'only the links the section uses');
  assert.match(s.troubleshooting, /Contact \[Datadog support\]\[5\]/);
  assert.match(s.furtherReading, /\[Blog\]\[6\]/);
});

test('a README without the usual headings is all overview', () => {
  assert.deepEqual(readmeSections('Just text.'), { overview: 'Just text.', setup: '', troubleshooting: '', furtherReading: '' });
});

test('Hugo shortcodes become plain Markdown', () => {
  assert.equal(resolveShortcodes('a {{< region-param key="dd_site" code="true" >}} b'), 'a `<OBSERVECRAFT_SITE>` b');
  assert.equal(resolveShortcodes('{{< site-region region="us,eu" >}}\nkept\n{{< /site-region >}}'), 'kept');
  assert.equal(resolveShortcodes('{{< site-region region="gov" >}}\ndropped\n{{< /site-region >}}'), '');
  assert.equal(resolveShortcodes('{{< get-metrics-from-git "postgres" >}}'), 'See the Data Collected tab.');
  assert.equal(resolveShortcodes('x {{< integration-api-key-picker >}} y'), 'x  y');
  assert.equal(resolveShortcodes('{{% collapse-content title="More" level="h4" %}}\nbody\n{{% /collapse-content %}}'), '**More**\n\nbody');
});

test('the HTML in READMEs becomes Markdown (the modal renders no raw HTML)', () => {
  assert.equal(htmlToMarkdown('<div class="alert alert-info">See <a href="https://docs.example.com/dbm" target="_blank">DBM</a>.</div>'), '> **Note:** See [DBM](https://docs.example.com/dbm).');
  assert.equal(htmlToMarkdown('<div class="alert alert-warning">\nCareful.\n</div>'), '> **Warning:** Careful.');
  assert.equal(htmlToMarkdown('a<br>b<br/>c'), 'a  \nb  \nc');
  assert.equal(htmlToMarkdown('<img src="https://example.com/x.png" alt="X" width="10">'), '![X](https://example.com/x.png)');
  assert.equal(htmlToMarkdown('<p>Para <strong>bold</strong> <code>x</code></p>'), 'Para **bold** `x`');
  assert.equal(htmlToMarkdown('keep `<div>` and\n```\n<tag>\n```'), 'keep `<div>` and\n```\n<tag>\n```', 'code stays as it is');
});

test('metadata.csv rows, with quoted descriptions and units', () => {
  const csv =
    'metric_name,metric_type,interval,unit_name,per_unit_name,description,orientation,integration,short_name,curated_metric,sample_tags\n' +
    'a.b,gauge,,byte,second,"Bytes, read ""fast"".\nSecond line",0,x,ab,,\n' +
    'a.c,count,10,,,Plain,0,x,ac,,';
  assert.deepEqual(metadataCsv(csv), [
    { name: 'a.b', type: 'gauge', unit: 'byte', perUnit: 'second', description: 'Bytes, read "fast".\nSecond line' },
    { name: 'a.c', type: 'count', description: 'Plain' },
  ]);
});

test('log facets of the integration pipeline', () => {
  const yaml =
    'id: x\nfacets:\n  - groups:\n      - Database\n    name: User\n    path: db.user\n    source: log\n  - facetType: range\n    groups:\n      - Measure\n    name: Duration\n    path: duration\n    source: log\npipeline:\n  type: pipeline\n';
  assert.deepEqual(logFacets(yaml), [
    { name: 'User', path: 'db.user', facetType: 'list', group: 'Database' },
    { name: 'Duration', path: 'duration', facetType: 'range', group: 'Measure' },
  ]);
});

test('monitor templates name their type', () => {
  const t = (type, query) => monitorTemplate({ title: 'T', description: 'D', definition: { type, query } });
  assert.deepEqual(t('query alert', 'avg(last_5m):avg:x{*} > 1'), { title: 'T', description: 'D', type: 'Metric Monitor', query: 'avg(last_5m):avg:x{*} > 1' });
  assert.equal(t('query alert', 'avg(last_1h):anomalies(avg:x{*}, "basic", 2) >= 1').type, 'Anomaly Monitor');
  assert.equal(t('log alert', 'logs("x").index("*").rollup("count").last("5m") > 1').type, 'Log Monitor');
  assert.equal(t('service check', '"x.can_connect".over("*").by("host").last(2).count_by_status()').type, 'Service Check Monitor');
});

test('the latest CHANGELOG entries', () => {
  const md = '# CHANGELOG - x\n\n<!-- towncrier release notes start -->\n\n## 2.0.0 / 2026-10-01\n\n***Added***:\n\n* New. ([#2](https://github.com/x/2))\n\n## 1.0.0 / 2026-01-01\n\n* First.\n';
  assert.deepEqual(changelog(md, 1), [{ version: '2.0.0', date: '2026-10-01', notes: '***Added***:\n\n* New. ([#2](https://github.com/x/2))' }]);
  assert.equal(changelog(md, 5).length, 2);
});

test('Postgres is imported with everything Datadog shows', () => {
  const file = new URL('../web/src/api/integrations/details/postgres.json', import.meta.url);
  assert.ok(existsSync(file), 'run node scripts/import-integration-details.mjs --source <integrations-core>');
  const d = JSON.parse(readFileSync(file, 'utf8'));
  assert.match(d.overview, /health and performance metrics for your Postgres database/);
  assert.match(d.setup, /create user datadog with password/);
  assert.ok(d.metrics.length >= 240, `${d.metrics.length} metrics`);
  assert.deepEqual(
    d.logFacets.map((f) => f.path),
    ['db.instance', 'db.operation', 'db.user', 'duration', 'postgres.session_id'],
  );
  assert.deepEqual(d.dashboards.map((x) => x.title).sort(), ['Postgres - Metrics', 'Postgres - Overview']);
  assert.deepEqual(d.monitors.map((m) => [m.title, m.type]).sort(), [
    ['Connection pool is reaching saturation point', 'Metric Monitor'],
    ['Replication delay is high', 'Anomaly Monitor'],
  ]);
  assert.ok(d.serviceChecks.some((c) => c.check === 'postgres.can_connect'));
  assert.ok(d.resources.some((r) => r.type === 'blog'));
  assert.ok(d.releaseNotes.length >= 5);
  assert.equal(d.source, `DataDog/integrations-core@${PINNED_REVISION} postgres`);
});
