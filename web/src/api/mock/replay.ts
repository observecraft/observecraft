// In-browser Session Replay for mock mode: recordings of the mock RUM sessions that have a replay,
// built from their events in the browser SDK's record format (the same fictional storefront pages as
// oc-tracegen's `replay.rs`), plus playlists, the watch history and heatmaps.
import type { Heatmap, HeatmapRequest, PlaylistSession, ReplayPlaylist, ReplayPlaylistInput, SessionReplay, WatchedSession } from '../rum/types';
import { mockEventsIn, sessionEvents, sessionRows } from './rum';

type Rec = Record<string, unknown>;
const delay = <T>(v: T, ms = 160): Promise<T> => new Promise((r) => setTimeout(() => r(structuredClone(v)), ms));
const fail = (m: string): never => {
  throw new Error(m);
};

export const WIDTH = 1280;
export const HEIGHT = 800;

const CSS =
  'body{margin:0;font-family:Helvetica,Arial,sans-serif;color:#1d2433;background:#f6f7f9}' +
  'header{position:absolute;left:0;top:0;width:1280px;height:64px;background:#14324a;color:#fff}' +
  '.logo{position:absolute;left:32px;top:18px;font-size:22px;font-weight:700}' +
  'nav{position:absolute;left:260px;top:22px}nav a{margin-right:28px;color:#cfe3f2}' +
  '.cart{position:absolute;left:1120px;top:20px;padding:4px 12px;border-radius:14px;background:#f2a93b;color:#14324a;font-weight:700}' +
  'main{position:absolute;left:0;top:64px;width:1280px;height:680px}' +
  'h1{position:absolute;left:64px;top:40px;margin:0;font-size:34px}' +
  '.lead{position:absolute;left:64px;top:96px;width:620px;color:#5b6577}' +
  '.cta{position:absolute;left:64px;top:170px;width:200px;height:46px;border:0;border-radius:6px;background:#2a7de1;color:#fff;font-size:16px}' +
  '.card{position:absolute;top:280px;width:260px;height:300px;border-radius:8px;background:#fff;box-shadow:0 1px 3px rgba(0,0,0,.12)}' +
  '.card .img{height:170px;border-radius:8px 8px 0 0;background:linear-gradient(135deg,#d8e6f3,#b9d3ea)}' +
  '.card p{margin:12px 16px}.price{font-weight:700}' +
  'input{position:absolute;left:64px;width:420px;height:36px;border:1px solid #c5cbd6;border-radius:4px;padding:0 10px}' +
  'footer{position:absolute;left:0;top:744px;width:1280px;height:56px;background:#e8ebf0;color:#5b6577;text-align:center;line-height:56px}';

/** (view names, title, lead, call to action, cards, form fields). */
const PAGES: [string[], string, string, string, number, number][] = [
  [['/', 'HomeScreen', '/invoices'], 'Spring sale on running gear', 'New season shoes and jackets, free returns for 60 days.', 'Shop now', 4, 0],
  [['/products', 'CatalogScreen', '/refunds'], 'Running shoes', '128 products · sorted by popularity', 'Filter: running shoes', 4, 0],
  [['/products/?', 'ProductScreen', '/invoices/?'], 'Trail Runner 3', 'Lightweight trail shoe with a grippy outsole. Sizes 38–47.', 'Add to cart', 0, 0],
  [['/cart', 'CartScreen'], 'Your cart', '2 items · free shipping over $50', 'Proceed to checkout', 2, 0],
  [['/checkout', 'CheckoutScreen'], 'Checkout', 'Shipping and payment', 'Place order', 0, 3],
];
const pageOf = (view: string) =>
  Math.max(
    0,
    PAGES.findIndex((p) => p[0].includes(view)),
  );
const ctaTop = (fields: number) => 64 + (fields ? 170 + fields * 56 + 10 : 170);

/** Meta, Focus and FullSnapshot records of a page, and the ids of its button, cart counter and inputs. */
function openView(page: number, href: string, at: number, cart: number): { records: Rec[]; cta: number; cartText: number; inputs: number[]; body: number } {
  const [, title, lead, ctaLabel, cards, fields] = PAGES[page];
  let next = 0;
  const id = () => ++next;
  const el = (tagName: string, attributes: Rec, childNodes: Rec[] = []): Rec => ({ type: 2, id: id(), tagName, attributes, childNodes });
  const text = (textContent: string): Rec => ({ type: 3, id: id(), textContent });
  const head = el('head', {}, [el('style', { _cssText: CSS }), el('title', {}, [text('Example Shop')])]);
  const cartText = text(String(cart));
  const header = el('header', {}, [
    el('a', { class: 'logo' }, [text('Example Shop')]),
    el(
      'nav',
      {},
      ['Products', 'Deals', 'Help'].map((l) => el('a', { href: '#' }, [text(l)])),
    ),
    el('a', { class: 'cart' }, [text('Cart '), cartText]),
  ]);
  const main: Rec[] = [el('h1', {}, [text(title)]), el('p', { class: 'lead' }, [text(lead)])];
  const inputs: number[] = [];
  ['Full name', 'Shipping address', 'Card number'].slice(0, fields).forEach((ph, i) => {
    const input = el('input', { type: 'text', placeholder: ph, style: `top:${170 + i * 56}px` });
    inputs.push(input.id as number);
    main.push(input);
  });
  const cta = el('button', { class: 'cta', style: `top:${ctaTop(fields) - 64}px` }, [text(ctaLabel)]);
  main.push(cta);
  for (let c = 0; c < cards; c++) {
    main.push(
      el('div', { class: 'card', style: `left:${64 + c * 290}px` }, [
        el('div', { class: 'img' }),
        el('p', {}, [text(['Trail Runner 3', 'Road Pro 2', 'Wind Shell', 'Merino Tee'][c % 4])]),
        el('p', { class: 'price' }, [text(['$129', '$149', '$89', '$45'][c % 4])]),
      ]),
    );
  }
  const body = el('body', {}, [header, el('main', {}, main), el('footer', {}, [text('© Example Shop · shop.example.com')])]);
  const html = el('html', { lang: 'en' }, [head, body]);
  const doc = { type: 0, id: 0, childNodes: [{ type: 1, id: id(), name: 'html', publicId: '', systemId: '' }, html] };
  return {
    records: [
      { type: 4, timestamp: at, data: { href, width: WIDTH, height: HEIGHT } },
      { type: 6, timestamp: at, data: { has_focus: true } },
      { type: 2, timestamp: at + 1, data: { node: doc, initialOffset: { top: 0, left: 0 } } },
    ],
    cta: cta.id as number,
    cartText: cartText.id as number,
    inputs,
    body: body.id as number,
  };
}

function hash(s: string): number {
  let h = 2166136261;
  for (const c of s) h = Math.imul(h ^ c.charCodeAt(0), 16777619);
  return h >>> 0;
}

/** A session's recording: one segment per view, with mouse moves towards its button and a click per action. */
function recording(id: string, startedAt: number) {
  const evs = sessionEvents(id, startedAt);
  const views = evs.filter((e) => e.type === 'view');
  let cart = 0;
  let rand = hash(id);
  const r = () => (rand = Math.imul(rand ^ (rand >>> 15), 2246822507) >>> 0) / 4294967296;
  return views.map((v, i) => {
    const end = views[i + 1]?.t ?? v.t + Number(v.attrs['view.time_spent'] ?? 20e9) / 1e6;
    const page = pageOf(v.attrs['view.name']);
    const opened = openView(page, v.attrs['view.url'] ?? 'https://shop.example.com/', v.t, cart);
    const records: Rec[] = [...opened.records];
    let mouse = [640, 400];
    const clicks = evs.filter((e) => e.type === 'action' && e.attrs['view.id'] === v.attrs['view.id']).map((e) => e.t);
    for (let t = v.t + 1500; t < end; t += 1500) {
      const clickNow = clicks.find((c) => c >= t - 1500 && c < t);
      const target = [164 + (r() - 0.5) * 120, ctaTop(opened.inputs.length) + 23 + (r() - 0.5) * 30];
      const positions = [1, 2, 3].map((k) => ({
        x: Math.round(mouse[0] + ((target[0] - mouse[0]) * k) / 3 + (r() - 0.5) * 30),
        y: Math.round(mouse[1] + ((target[1] - mouse[1]) * k) / 3 + (r() - 0.5) * 30),
        id: opened.body,
        timeOffset: -(3 - k) * 400,
      }));
      records.push({ type: 3, timestamp: t, data: { source: 1, positions } });
      mouse = target;
      opened.inputs.forEach((input, n) => r() < 0.3 && records.push({ type: 3, timestamp: t + 100 + n, data: { source: 5, id: input, text: '*'.repeat(6 + n * 4) } }));
      if (clickNow !== undefined) {
        for (const [kind, dt] of [
          [1, 0],
          [0, 80],
          [2, 81],
        ])
          records.push({ type: 3, timestamp: t + 200 + dt, data: { source: 2, type: kind, id: opened.cta, x: Math.round(target[0]), y: Math.round(target[1]) } });
        if (page === 2) {
          cart++;
          records.push({ type: 3, timestamp: t + 400, data: { source: 0, adds: [], removes: [], attributes: [], texts: [{ id: opened.cartText, value: String(cart) }] } });
        }
      }
    }
    records.push({ type: 7, timestamp: end });
    return { viewId: v.attrs['view.id'], start: v.t, end, records };
  });
}

// ---------------------------------------------------------------- recordings and watches

const watches = new Map<string, WatchedSession>();

export async function getSessionReplay(id: string, startedAt: number): Promise<SessionReplay> {
  const evs = sessionEvents(id, startedAt);
  if (!evs.length || evs[0].attrs['session.has_replay'] !== 'true') fail('No recording of this session');
  const session = sessionRows(evs)[0] ?? null;
  const events = evs
    .filter((e) => !['resource', 'long_task'].includes(e.type))
    .map((e, i) => ({ id: e.type === 'view' ? e.attrs['view.id'] : `${e.t}-${i}`, timestamp: e.t, type: e.type, service: e.service, attributes: { ...e.attrs } }));
  return delay({ session, segments: recording(id, startedAt) as unknown as SessionReplay['segments'], events, watched: watches.has(id) ? 1 : 0 });
}

export async function markReplayWatched(id: string, startedAt: number): Promise<void> {
  watches.set(id, { sessionId: id, startedAt, watchedAt: Date.now() });
  return delay(undefined, 40);
}

export async function replayHistory(): Promise<WatchedSession[]> {
  return delay([...watches.values()].sort((a, b) => b.watchedAt - a.watchedAt));
}

// ---------------------------------------------------------------- playlists

interface MockPlaylist extends Omit<ReplayPlaylist, 'sessionCount' | 'sessions'> {
  sessions: PlaylistSession[];
}

const now0 = Date.now();
let playlists: MockPlaylist[] = [
  {
    id: 'a1b2c3d4-0000-4000-8000-00000000c001',
    name: 'Checkout errors',
    description: 'Sessions where placing the order failed',
    createdBy: 'Priya Shah',
    createdByMe: false,
    createdAt: now0 - 9 * 86_400_000,
    updatedAt: now0 - 86_400_000,
    sessions: [],
  },
  {
    id: 'a1b2c3d4-0000-4000-8000-00000000c002',
    name: 'Rage clicks',
    description: '',
    createdBy: 'Alex Morgan',
    createdByMe: true,
    createdAt: now0 - 3 * 86_400_000,
    updatedAt: now0 - 3 * 86_400_000,
    sessions: [],
  },
];

/** The first playlists get a few of the latest recorded sessions. */
function seedPlaylists() {
  if (playlists.some((p) => p.sessions.length)) return;
  const t = Date.now();
  const recorded = sessionRows(mockEventsIn(t - 3_600_000, t)).filter((s) => s.attributes['session.has_replay'] === 'true');
  playlists[0].sessions = recorded
    .filter((s) => s.attributes['session.error.count'] !== '0')
    .slice(0, 5)
    .map((s) => ({ sessionId: s.id, startedAt: s.timestamp }));
  playlists[1].sessions = recorded.slice(0, 1).map((s) => ({ sessionId: s.id, startedAt: s.timestamp }));
}

const view = (p: MockPlaylist, withSessions: boolean): ReplayPlaylist => ({ ...p, sessionCount: p.sessions.length, sessions: withSessions ? p.sessions : [] });

export async function listReplayPlaylists(): Promise<ReplayPlaylist[]> {
  seedPlaylists();
  return delay([...playlists].sort((a, b) => b.updatedAt - a.updatedAt).map((p) => view(p, false)));
}

export async function getReplayPlaylist(id: string): Promise<ReplayPlaylist> {
  seedPlaylists();
  return delay(view(playlists.find((p) => p.id === id) ?? fail('Playlist not found'), true));
}

export async function createReplayPlaylist(input: ReplayPlaylistInput): Promise<ReplayPlaylist> {
  if (!input.name.trim()) fail('A playlist needs a name (at most 200 characters)');
  const p: MockPlaylist = {
    id: crypto.randomUUID(),
    name: input.name.trim(),
    description: input.description ?? '',
    createdBy: 'Alex Morgan',
    createdByMe: true,
    createdAt: Date.now(),
    updatedAt: Date.now(),
    sessions: [],
  };
  playlists = [p, ...playlists];
  return delay(view(p, true));
}

export async function updateReplayPlaylist(id: string, input: ReplayPlaylistInput): Promise<ReplayPlaylist> {
  const p = playlists.find((x) => x.id === id) ?? fail('Playlist not found');
  if (!input.name.trim()) fail('A playlist needs a name (at most 200 characters)');
  Object.assign(p, { name: input.name.trim(), description: input.description ?? '', updatedAt: Date.now() });
  return delay(view(p, true));
}

export async function deleteReplayPlaylist(id: string): Promise<void> {
  playlists.find((x) => x.id === id) ?? fail('Playlist not found');
  playlists = playlists.filter((p) => p.id !== id);
  return delay(undefined);
}

export async function addPlaylistSession(id: string, s: PlaylistSession): Promise<ReplayPlaylist> {
  const p = playlists.find((x) => x.id === id) ?? fail('Playlist not found');
  if (!p.sessions.some((x) => x.sessionId === s.sessionId)) p.sessions = [s, ...p.sessions];
  p.updatedAt = Date.now();
  return delay(view(p, true));
}

export async function removePlaylistSession(id: string, sessionId: string): Promise<void> {
  const p = playlists.find((x) => x.id === id) ?? fail('Playlist not found');
  p.sessions = p.sessions.filter((x) => x.sessionId !== sessionId);
  return delay(undefined);
}

// ---------------------------------------------------------------- heatmaps

export async function replayHeatmap(req: HeatmapRequest): Promise<Heatmap> {
  const evs = mockEventsIn(req.range.from, req.range.to).filter((e) => !req.applicationId || e.attrs['application.id'] === req.applicationId);
  const byView = new Map<string, Set<string>>();
  for (const e of evs.filter((e) => e.type === 'view')) byView.set(e.attrs['view.name'], (byView.get(e.attrs['view.name']) ?? new Set()).add(e.attrs['session.id']));
  const views = [...byView.entries()]
    .map(([name, s]) => ({ name, count: s.size }))
    .sort((a, b) => b.count - a.count)
    .slice(0, 10);
  const out: Heatmap = { views, clicks: [], topElements: [], snapshot: null };
  if (!req.viewName) return delay(out);
  const actions = evs.filter((e) => e.type === 'action' && e.attrs['view.name'] === req.viewName && (!req.frustration || e.attrs['action.frustration.type'] === req.frustration));
  const top = new Map<string, number>();
  for (const a of actions) top.set(a.attrs['action.target.name'], (top.get(a.attrs['action.target.name']) ?? 0) + 1);
  out.topElements = [...top.entries()].map(([name, count]) => ({ name, count })).sort((a, b) => b.count - a.count);
  const page = pageOf(req.viewName);
  out.snapshot = openView(page, `https://shop.example.com${req.viewName.replace('?', 'sku-1042')}`, req.range.to, 0).records.filter((r) => r.type !== 6) as unknown as Heatmap['snapshot'];
  let seed = hash(req.viewName);
  const r = () => (seed = Math.imul(seed ^ (seed >>> 15), 2246822507) >>> 0) / 4294967296;
  const fields = PAGES[page][5];
  out.clicks = actions.slice(0, 400).map(() => ({ x: Math.round(164 + (r() - 0.5) * 150), y: Math.round(ctaTop(fields) + 23 + (r() - 0.5) * 40) }));
  return delay(out);
}
