// Text placed in HTML that charts render (ECharts tooltip formatters): web/src/lib/html.ts.
import assert from 'node:assert/strict';
import test from 'node:test';
import { escapeHtml } from '../web/src/lib/html.ts';

test('text in a tooltip is escaped, so a service named like markup stays text', () => {
  assert.equal(escapeHtml(`<img src=x onerror="a('b')"> & co`), '&lt;img src=x onerror=&quot;a(&#39;b&#39;)&quot;&gt; &amp; co');
  assert.equal(escapeHtml('plain'), 'plain');
});
