// The official AWS, Azure and Google Cloud service icons (web/scripts/fetch-cloud-icons.mjs).
import assert from 'node:assert/strict';
import { existsSync, readFileSync, readdirSync } from 'node:fs';
import test from 'node:test';
import { OUT, inlineStyles, services } from '../web/scripts/fetch-cloud-icons.mjs';

test('class styles become inline styles, so icons never restyle each other', () => {
  const svg = '<svg viewBox="0 0 2 2"><defs><style>.st0{fill:none}.st1,.st2{fill:#4285f4;}</style></defs><path class="st1" d="M0"/><rect class="st0 st2" style="opacity:.5"/></svg>';
  assert.equal(inlineStyles(svg), '<svg viewBox="0 0 2 2"><defs></defs><path style="fill:#4285f4" d="M0"/><rect style="fill:none;fill:#4285f4;opacity:.5"/></svg>');
});

test('almost every cloud service has its provider’s icon, stored without styles or scripts', () => {
  for (const cloud of ['aws', 'azure', 'gcp']) {
    const all = services(cloud);
    const present = all.filter((s) => existsSync(new URL(`${cloud}/${s.id}.svg`, OUT)));
    assert.ok(present.length >= all.length - 3, `${cloud}: ${present.length} of ${all.length}`);
  }
  for (const cloud of ['aws', 'azure', 'gcp'])
    for (const file of readdirSync(new URL(`${cloud}/`, OUT))) {
      const svg = readFileSync(new URL(`${cloud}/${file}`, OUT), 'utf8');
      assert.match(svg, /^<svg[^>]* viewBox="[^"]+"/, file);
      assert.doesNotMatch(svg, /<style|<script|class="|on[a-z]+=/i, file);
    }
  assert.ok(existsSync(new URL('NOTICE.md', OUT)), 'the providers’ terms are kept with the icons');
});
