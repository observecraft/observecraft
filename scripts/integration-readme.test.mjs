// README sections in the integration modal (web/src/pages/integrations/readme.ts).
import assert from 'node:assert/strict';
import test from 'node:test';
import { readmeSegments } from '../web/src/pages/integrations/readme.ts';

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
