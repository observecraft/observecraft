// The browser SDK's compact "change" recording format (full snapshots with `format: 1`, records of type 12),
// read into the records the player rebuilds pages from: web/src/pages/rum/replay/changeFormat.ts.
import assert from 'node:assert/strict';
import test from 'node:test';
import { fromChangeFormat } from '../web/src/pages/rum/replay/changeFormat.ts';

const CSS = '.cart { color: red; }';

// Strings 0-19 of the table, in the order the SDK defines them (one id space across roles).
const STRINGS = [
  11,
  [1, '#document', '#doctype', 'HTML', 'HEAD', 'LINK', 'BODY', 'DIV', '#text', 'svg>svg', 'svg>linearGradient'],
  [2, 'lang', 'rel', 'class'],
  [3, 'en', 'stylesheet', 'cart', 'html'],
  [4, 'Checkout'],
  [6, CSS],
  [7, ''],
];

// document(0) > doctype(1), html(2) > head(3) > link(4); body(5) > div.cart(6) > "Checkout"(7), svg(8) > linearGradient(9)
const snapshot = (timestamp = 1000) => ({
  type: 2,
  format: 1,
  timestamp,
  data: [STRINGS, [1, [null, 0], [1, 1, 16, 19, 19], [0, 2, [10, 13]], [1, 3], [1, 4, [11, 14]], [3, 5], [1, 6, [12, 15]], [1, 7, 17], [0, 8], [1, 9]], [7, [18]], [8, [4, 0]], [6, [0, 0, 120]]],
});

const el = (id, tagName, attributes = {}, childNodes = [], isSVG = undefined) => ({ type: 2, id, tagName, attributes, childNodes, ...(isSVG ? { isSVG } : {}) });
const mutation = (m) => ({ source: 0, adds: [], removes: [], texts: [], attributes: [], ...m });

test('a full snapshot in the change format becomes the page tree, with its stylesheets and scroll positions after it', () => {
  const out = fromChangeFormat([snapshot()]);
  assert.deepEqual(out[0], {
    type: 2,
    timestamp: 1000,
    data: {
      node: {
        type: 0,
        id: 0,
        childNodes: [
          { type: 1, id: 1, name: 'html' },
          el(2, 'html', { lang: 'en' }, [
            el(3, 'head', {}, [el(4, 'link', { rel: 'stylesheet' })]),
            el(5, 'body', {}, [el(6, 'div', { class: 'cart' }, [{ type: 3, id: 7, textContent: 'Checkout' }, el(8, 'svg', {}, [el(9, 'linearGradient', {}, [], true)], true)])]),
          ]),
        ],
      },
    },
  });
  assert.deepEqual(out.slice(1), [
    { type: 3, timestamp: 1000, data: mutation({ attributes: [{ id: 4, attributes: { _cssText: CSS } }] }) },
    { type: 3, timestamp: 1000, data: { source: 3, id: 0, x: 0, y: 120 } },
  ]);
});

test('changes after the snapshot become mutations, inputs and scrolls, with node ids continuing from the snapshot', () => {
  const change = {
    type: 12,
    timestamp: 2000,
    data: [
      [11, [1, 'SPAN'], [4, '2 items'], [2, 'data-state'], [3, 'open']], // strings 20-23
      [1, [-2, 20], [0, 6], [1, 7, 21]], // span(10) before svg(8); div(11) after it; "2 items"(12) in div(11)
      [3, [6, [22, 23]], [6, [12]]],
      [4, [7, 21]],
      [2, 9],
      [12, [6, 21]],
      [13, [0, 6, 10]],
      [6, [6, 0, 40]],
      [10, [0, 0, 0, 0, 1290, 930, 1]],
    ],
  };
  const out = fromChangeFormat([snapshot(), change]).filter((r) => r.timestamp === 2000);
  assert.deepEqual(out, [
    {
      type: 3,
      timestamp: 2000,
      data: mutation({
        adds: [
          { parentId: 6, nextId: 8, node: el(10, 'span') },
          { parentId: 6, nextId: 8, node: el(11, 'div') },
          { parentId: 11, nextId: null, node: { type: 3, id: 12, textContent: '2 items' } },
        ],
      }),
    },
    {
      type: 3,
      timestamp: 2000,
      data: mutation({
        attributes: [
          { id: 6, attributes: { 'data-state': 'open' } },
          { id: 6, attributes: { class: null } },
        ],
      }),
    },
    { type: 3, timestamp: 2000, data: mutation({ texts: [{ id: 7, value: '2 items' }] }) },
    { type: 3, timestamp: 2000, data: mutation({ removes: [{ id: 9 }] }) },
    { type: 3, timestamp: 2000, data: { source: 5, id: 6, text: '2 items' } },
    { type: 3, timestamp: 2000, data: { source: 5, id: 6, isChecked: true } },
    { type: 3, timestamp: 2000, data: { source: 5, id: 10, isChecked: true } },
    { type: 3, timestamp: 2000, data: { source: 3, id: 6, x: 0, y: 40 } },
  ]);
});

test('a node added after a removed sibling goes where the sibling list now ends', () => {
  const out = fromChangeFormat([
    snapshot(),
    {
      type: 12,
      timestamp: 2000,
      data: [
        [2, 8],
        [1, [4, 6]],
      ],
    },
  ]); // remove svg(8), then a div(10) in div(6)
  assert.deepEqual(out.at(-1).data.adds, [{ parentId: 6, nextId: null, node: el(10, 'div') }]);
});

test('each full snapshot starts a new string table, and ClearStrings empties it', () => {
  const second = {
    type: 2,
    format: 1,
    timestamp: 3000,
    data: [
      [11, [1, '#document', 'HTML'], [3, 'fr']],
      [1, [null, 0], [1, 1]],
    ],
  };
  const cleared = { type: 12, timestamp: 4000, data: [[14], [11, [3, 'checkout']], [3, [1, ['lang', 0]]]] };
  const out = fromChangeFormat([snapshot(), second, cleared]);
  assert.deepEqual(out.find((r) => r.timestamp === 3000).data.node, { type: 0, id: 0, childNodes: [el(1, 'html')] });
  assert.deepEqual(out.at(-1).data.attributes, [{ id: 1, attributes: { lang: 'checkout' } }]);
});

test('change records without the snapshot they build on are dropped until the next full snapshot', () => {
  const orphan = { type: 12, timestamp: 500, data: [[1, [1, 30]]] };
  const focus = { type: 6, timestamp: 600, data: { has_focus: true } };
  const out = fromChangeFormat([orphan, focus, snapshot()]);
  assert.deepEqual(
    out.map((r) => r.timestamp),
    [600, 1000, 1000, 1000],
  );
});

test('stylesheets adopted by the document are added to its head', () => {
  const rec = { ...snapshot(), data: [...snapshot().data.slice(0, 3), [8, [0, 0]]] };
  const out = fromChangeFormat([rec]);
  assert.deepEqual(out[1].data.adds, [{ parentId: 3, nextId: null, node: el(-1, 'style', { _cssText: CSS }) }]);
});

test("a shadow root's children are shown in its host", () => {
  const rec = {
    type: 2,
    format: 1,
    timestamp: 1000,
    data: [
      [11, [1, '#document', 'HTML', 'BODY', 'DIV', '#shadow-root', '#text'], [4, 'In stock']],
      [1, [null, 0], [1, 1], [1, 2], [1, 3], [1, 4], [1, 5, 6]],
    ],
  };
  const [snap] = fromChangeFormat([rec]);
  assert.deepEqual(snap.data.node.childNodes[0].childNodes[0], el(2, 'body', {}, [el(3, 'div', {}, [{ type: 3, id: 5, textContent: 'In stock' }])]));
});

test('records in the older format are kept as they are', () => {
  const records = [
    { type: 4, timestamp: 1, data: { href: 'https://shop.example.com/', width: 1280, height: 800 } },
    { type: 2, timestamp: 2, data: { node: { type: 0, id: 0, childNodes: [] } } },
    { type: 3, timestamp: 3, data: { source: 1, positions: [{ x: 5, y: 5, id: 1, timeOffset: 0 }] } },
  ];
  assert.deepEqual(fromChangeFormat(records), records);
});
