// README sections in the integration modal (web/src/pages/integrations/readme.ts).
import assert from 'node:assert/strict';
import test from 'node:test';
import { readFileSync } from 'node:fs';
import { forObserveCraft, readmeSegments } from '../web/src/pages/integrations/readme.ts';

const SETUP = `### Installation

See [the Agent][1].

<!-- xxx tabs xxx -->
<!-- xxx tab "Host" xxx -->
On a [host][2].
<!-- xxz tab xxx -->
<!-- xxx tab "Docker" xxx -->
In a container.
<!-- xxz tab xxx -->
<!-- xxz tabs xxx -->

### Validation

Run the status command.

[1]: https://docs.example.com/agent
[2]: https://docs.example.com/host`;

test('tab markers become tab groups between Markdown parts', () => {
  const s = readmeSegments(SETUP);
  assert.deepEqual(
    s.map((x) => (x.kind === 'tabs' ? x.tabs.map((t) => t.name) : x.kind)),
    ['markdown', ['Host', 'Docker'], 'markdown'],
  );
  assert.match(s[1].tabs[0].markdown, /^On a \[host\]\[2\]\./);
  assert.match(s[2].markdown, /^### Validation/);
});

test('every part keeps the link definitions, so its reference links resolve', () => {
  for (const part of readmeSegments(SETUP).flatMap((x) => (x.kind === 'tabs' ? x.tabs : [x]))) assert.match(part.markdown, /\[2\]: https:\/\/docs\.example\.com\/host$/);
});

test('Markdown without tabs is one part; HTML comments are dropped', () => {
  assert.deepEqual(readmeSegments('Text <!-- note -->.'), [{ kind: 'markdown', markdown: 'Text .' }]);
  assert.deepEqual(readmeSegments(''), []);
});

test('HTML comments are removed however they are nested', () => {
  const [part] = readmeSegments('Text <!<!-- a -->-- b --> after');
  assert.doesNotMatch(part.markdown, /<!--/);
});

test('setup instructions point at ObserveCraft, not at Datadog’s app; code and the Agent’s name stay', () => {
  const md = [
    'Install the [Datadog Agent][2], then check the [Live Tail][7] in Datadog. Metrics sent to Datadog appear in your Datadog account.',
    'Update the JSON with your [Datadog API Key][41]. Navigate to the [Monitors](https://app.datadoghq.eu/monitors/manage) page in the Datadog app.',
    'The Datadog Agent ships to Datadog’s intake: `curl https://app.datadoghq.com/intake/webhook/zabbix?api_key=x`.',
    'Contact your Datadog account team.',
    '[2]: https://app.datadoghq.com/account/settings/agent/latest',
    '[7]: https://app.datadoghq.com/logs/livetail',
    '[9]: https://app.datadoghq.com/some/unknown/page?x=1',
  ].join('\n');
  assert.equal(
    forObserveCraft(md),
    [
      'Install the [Datadog Agent][2], then check the [Live Tail][7] in ObserveCraft. Metrics sent to ObserveCraft appear in ObserveCraft.',
      'Update the JSON with your [ObserveCraft ingest key][41]. Navigate to the [Monitors](/monitors) page in ObserveCraft.',
      'The Datadog Agent ships to Datadog’s intake: `curl https://app.datadoghq.com/intake/webhook/zabbix?api_key=x`.',
      'Contact your Datadog account team.',
      '[2]: /infrastructure/add',
      '[7]: /logs/livetail',
      '[9]: /',
    ].join('\n'),
  );
});

test('README parts link our pages, reference definitions included', () => {
  const [part] = readmeSegments('Check the [Live Tail][7] in Datadog.\n\n[7]: https://app.datadoghq.com/logs/livetail');
  assert.equal(part.markdown, 'Check the [Live Tail][7] in ObserveCraft.\n\n[7]: /logs/livetail');
});

test('every Agent integration’s setup is free of Datadog app links and “in Datadog” steps', () => {
  const catalog = JSON.parse(readFileSync(new URL('../web/src/api/integrations/catalog.generated.json', import.meta.url), 'utf8')).integrations;
  const left = [];
  for (const c of catalog.filter((c) => c.collection === 'agent')) {
    const { setup } = JSON.parse(readFileSync(new URL(`../web/src/api/integrations/details/${c.id}.json`, import.meta.url), 'utf8'));
    assert.ok(setup.trim(), `${c.id} has setup instructions`);
    // Outside code: what a reader is told to do.
    const prose = forObserveCraft(setup)
      .split(/(```[\s\S]*?```|`[^`\n]*`)/)
      .filter((_, i) => i % 2 === 0)
      .join(' ');
    for (const m of prose.matchAll(/https?:\/\/(?:app|us3|us5|ap1|ap2)\.datadoghq\.(?:com|eu)\S*|\b(?:in|to) (?:your )?Datadog\b(?! Agent| Cluster|’s|'s)|Datadog API key/gi))
      left.push(`${c.id}: ${m[0]}`);
  }
  assert.deepEqual(left, []);
});
