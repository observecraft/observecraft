// In-browser Assist for mock mode: scripted replies over the demo domain (no model), streamed like the
// server's (routes/assist.rs): a tool or two, then the answer word by word; asking to create a monitor
// pauses for approval. Conversations, settings and usage live in memory for the session.
import type { AssistEvent, AssistReport, AssistReportRequest, AssistSettings, AssistStatus, ChatRequest, ConversationSummary, ConversationView, ViewMessage, ViewTool } from '../assist/types';

const SKILLS: AssistStatus['skills'] = [
  { name: 'logs', description: 'Search log events and find error patterns in them.', tools: ['search_logs'], writes: false },
  { name: 'metrics', description: 'Query metrics with the metric query syntax (avg:system.cpu.user{env:prod} by {host}).', tools: ['query_metrics'], writes: false },
  { name: 'apm', description: 'Service health (requests, errors, latency) and traces of instrumented services.', tools: ['list_services', 'search_traces'], writes: false },
  { name: 'monitors', description: 'Find monitors and their state, and create new ones (with your approval).', tools: ['search_monitors', 'get_monitor', 'create_monitor'], writes: true },
  { name: 'incidents', description: 'Find incidents and declare new ones (with your approval).', tools: ['search_incidents', 'get_incident', 'declare_incident'], writes: true },
  { name: 'errors', description: 'Error Tracking issues: the errors services raise, grouped.', tools: ['search_error_issues'], writes: false },
  { name: 'kubernetes', description: 'Kubernetes clusters, nodes, workloads and pods.', tools: ['search_kubernetes'], writes: false },
];

interface Conversation {
  id: string;
  title: string;
  surface: string;
  updatedAt: number;
  messages: ViewMessage[];
  pending: ViewTool[];
}

interface UsageRow {
  at: number;
  user: string;
  surface: string;
  newConversation: boolean;
}

const DAY = 86_400_000;
let conversations: Conversation[] | null = null;
let settings: AssistSettings = { context: '', disabledSkills: [] };
let usage: UsageRow[] | null = null;

const uuid = () => (globalThis.crypto?.randomUUID ? globalThis.crypto.randomUUID() : `id-${Math.random().toString(36).slice(2)}`);
const sleep = (ms: number, signal: AbortSignal) =>
  new Promise<void>((resolve, reject) => {
    const t = setTimeout(resolve, ms);
    signal.addEventListener('abort', () => (clearTimeout(t), reject(new DOMException('Aborted', 'AbortError'))), { once: true });
  });

function db(): Conversation[] {
  if (conversations) return conversations;
  const now = Date.now();
  const seeded: Conversation[] = [
    {
      id: uuid(),
      title: 'Why did checkout latency spike this morning?',
      surface: 'chat',
      updatedAt: now - 3 * 3_600_000,
      pending: [],
      messages: [
        { role: 'user', text: 'Why did checkout latency spike this morning?', tools: [] },
        {
          role: 'assistant',
          text: '**order-service** p95 latency rose from 180 ms to 2.4 s between 07:10 and 07:40 UTC. Its spans show `inventory-service` calls timing out after a deploy of **inventory-service 3.2.1** at 07:08.\n\n- 412 `TimeoutError` logs on order-service in that window ([logs](/logs?query=service%3Aorder-service%20status%3Aerror))\n- Latency recovered at 07:41, when the deploy was rolled back.',
          tools: [
            { id: 't1', name: 'list_services', input: { from: '6h' } as unknown as ViewTool['input'], status: 'ok', summary: '[{"service":"order-service","p95":2400,…}]' },
            { id: 't2', name: 'search_logs', input: { query: 'service:order-service status:error', from: '6h' } as unknown as ViewTool['input'], status: 'ok', summary: '{"total":412,"logs":[…]}' },
          ],
        },
      ],
    },
  ];
  conversations = seeded;
  return seeded;
}

function usageRows(): UsageRow[] {
  if (usage) return usage;
  const now = Date.now();
  const users = ['me', 'u2', 'u3', 'u4'];
  usage = [];
  let seed = 11;
  const rand = () => (seed = (seed * 1_103_515_245 + 12_345) >>> 0) / 4_294_967_296;
  for (let d = 30; d >= 0; d--) {
    const n = Math.floor(rand() * 9);
    for (let i = 0; i < n; i++)
      usage.push({ at: now - d * DAY - Math.floor(rand() * DAY), user: users[Math.floor(rand() * users.length)], surface: rand() < 0.7 ? 'chat' : 'panel', newConversation: rand() < 0.35 });
  }
  return usage;
}

export async function status(): Promise<AssistStatus> {
  return { configured: true, provider: 'anthropic', model: 'claude-opus-5', skills: SKILLS };
}

export async function getSettings(): Promise<AssistSettings> {
  return structuredClone(settings);
}

export async function updateSettings(s: AssistSettings): Promise<void> {
  if (s.context.length > 4000) throw new Error('The user context is limited to 4000 characters');
  const unknown = s.disabledSkills.find((d) => !SKILLS.some((k) => k.name === d));
  if (unknown) throw new Error(`Unknown skill ${unknown}`);
  settings = { context: s.context.trim(), disabledSkills: [...s.disabledSkills] };
}

export async function listConversations(): Promise<ConversationSummary[]> {
  return db()
    .slice()
    .sort((a, b) => b.updatedAt - a.updatedAt)
    .map((c) => ({ id: c.id, title: c.title, surface: c.surface, updatedAt: c.updatedAt }));
}

function find(id: string): Conversation {
  const c = db().find((x) => x.id === id);
  if (!c) throw new Error('Conversation not found');
  return c;
}

export async function getConversation(id: string): Promise<ConversationView> {
  const c = find(id);
  return { id: c.id, title: c.title, surface: c.surface, messages: structuredClone(c.messages), awaitingApproval: c.pending.length > 0 };
}

export async function deleteConversation(id: string): Promise<void> {
  find(id);
  conversations = db().filter((c) => c.id !== id);
}

interface Script {
  tools: ViewTool[];
  answer: string;
  /** A write that waits for approval, and the answer once it ran. */
  write?: { tool: ViewTool; answer: string };
}

function script(message: string): Script {
  const m = message.toLowerCase();
  const tool = (name: string, input: Record<string, unknown>, summary: string): ViewTool => ({ id: uuid(), name, input: input as unknown as ViewTool['input'], status: 'ok', summary });
  const off = (skill: string) => settings.disabledSkills.includes(skill);
  if (/monitor/.test(m) && /(create|add|set up|setup|make)/.test(m) && !off('monitors')) {
    return {
      tools: [tool('search_monitors', { query: 'checkout' }, '{"monitors":[],"total":0}')],
      answer: "There's no monitor on checkout errors yet. I can create one that alerts when **order-service** logs more than 10 errors in 5 minutes:",
      write: {
        tool: {
          ...tool(
            'create_monitor',
            { name: 'High error rate on service:order-service', type: 'log', query: 'logs("service:order-service status:error") by {service}', comparator: 'above', critical: 10, window_minutes: 5 },
            '',
          ),
          status: 'pending',
        },
        answer: 'Created **High error rate on service:order-service** ([monitor](/monitors)). It evaluates every minute over the last 5 minutes and alerts above 10 errors.',
      },
    };
  }
  if (/incident/.test(m) && !off('incidents')) {
    return {
      tools: [tool('search_incidents', { from: '30d' }, '{"incidents":[{"id":97,"title":"Orders stuck in pending",…}]}')],
      answer:
        'Two incidents in the last 30 days:\n\n| Incident | Severity | Status | Duration |\n|---|---|---|---|\n| [IR-97 Orders stuck in pending](/incidents/97) | SEV-2 | Resolved | 1h 12m |\n| [IR-98 Card authorizations failing in eu-west-1](/incidents/98) | SEV-1 | Active | 34m so far |\n\nIR-98 is still active: **billing** returns 502 for 6% of card authorizations.',
    };
  }
  if (/(log|error|fail|exception)/.test(m) && !off('logs')) {
    return {
      tools: [tool('search_logs', { query: 'status:error', from: '1h' }, '{"total":1284,"logs":[…]}'), tool('search_error_issues', { from: '1h' }, '{"issues":[…]}')],
      answer:
        'The most frequent errors in the last hour:\n\n1. **order-service** `TimeoutError: inventory-service did not answer within 2s`: 612 logs, since 09:12.\n2. **billing** `PaymentGatewayError: 502 from provider`: 402 logs.\n3. **storefront-api** `ValueError: invalid coupon code`: 270 logs, steady all day (likely user input).\n\nThe first two started within a minute of each other, which points at a shared dependency. [Open the logs](/logs?query=status%3Aerror).',
    };
  }
  return {
    tools: [tool('list_services', { from: '1h' }, '[{"service":"api-gateway","requests":…}]')],
    answer:
      'Services in the last hour:\n\n| Service | Requests/s | Error rate | p95 |\n|---|---|---|---|\n| api-gateway | 412 | 0.4% | 120 ms |\n| storefront-api | 280 | 1.1% | 340 ms |\n| order-service | 96 | **6.2%** | **2.1 s** |\n| billing | 41 | 2.4% | 610 ms |\n\n**order-service** stands out: its error rate and latency are well above the others.',
  };
}

/** Writes the answer word by word. */
async function streamText(text: string, emit: (e: AssistEvent) => void, signal: AbortSignal) {
  for (const word of text.match(/^\s+|\S+\s*/g) ?? []) {
    await sleep(18, signal);
    emit({ type: 'text', text: word });
  }
}

async function runTools(tools: ViewTool[], emit: (e: AssistEvent) => void, signal: AbortSignal) {
  for (const t of tools) {
    emit({ type: 'tool_start', id: t.id, name: t.name, input: t.input });
    await sleep(350, signal);
    emit({ type: 'tool_end', id: t.id, ok: t.status !== 'error', summary: t.summary });
  }
}

export async function chat(req: ChatRequest, emit: (e: AssistEvent) => void, signal: AbortSignal): Promise<void> {
  const text = req.message.trim();
  if (!text) throw new Error('A message has 1 to 20000 characters');
  let c: Conversation;
  if (req.conversationId) {
    c = find(req.conversationId);
    if (c.pending.length) throw new Error('Approve or decline the pending actions first');
  } else {
    c = { id: uuid(), title: text.split('\n')[0].slice(0, 80), surface: req.surface ?? 'chat', updatedAt: Date.now(), messages: [], pending: [] };
    db().push(c);
  }
  usageRows().push({ at: Date.now(), user: 'me', surface: c.surface, newConversation: !req.conversationId });
  c.messages.push({ role: 'user', text, tools: [] });
  emit({ type: 'conversation', id: c.id, title: c.title });
  const s = script(text);
  const answer: ViewMessage = { role: 'assistant', text: '', tools: [] };
  c.messages.push(answer);
  await runTools(s.tools, emit, signal);
  answer.tools.push(...structuredClone(s.tools));
  await streamText(s.answer, emit, signal);
  answer.text = s.answer;
  if (s.write && !req.autoApprove) {
    c.pending = [s.write.tool];
    answer.tools.push(structuredClone(s.write.tool));
    emit({ type: 'approval', id: s.write.tool.id, name: s.write.tool.name, input: s.write.tool.input });
    pendingAnswers.set(c.id, s.write.answer);
  } else if (s.write) {
    await runTools([{ ...s.write.tool, status: 'ok', summary: '{"id":42,"name":"High error rate on service:order-service"}' }], emit, signal);
    answer.tools.push({ ...s.write.tool, status: 'ok', summary: '{"id":42}' });
    await streamText(`\n\n${s.write.answer}`, emit, signal);
    answer.text += `\n\n${s.write.answer}`;
  }
  c.updatedAt = Date.now();
  emit({ type: 'done', awaiting_approval: c.pending.length > 0, input_tokens: 1800, output_tokens: 240 });
}

const pendingAnswers = new Map<string, string>();

export async function approve(id: string, approved: string[], emit: (e: AssistEvent) => void, signal: AbortSignal): Promise<void> {
  const c = find(id);
  if (!c.pending.length) throw new Error('Nothing is waiting for approval');
  const answer = c.messages[c.messages.length - 1];
  for (const p of c.pending) {
    const ok = approved.includes(p.id);
    const t = answer.tools.find((x) => x.id === p.id);
    emit({ type: 'tool_start', id: p.id, name: p.name, input: p.input });
    await sleep(400, signal);
    const summary = ok ? '{"id":42,"name":"High error rate on service:order-service"}' : 'The user declined this action.';
    emit({ type: 'tool_end', id: p.id, ok, summary });
    if (t) Object.assign(t, { status: ok ? 'ok' : 'error', summary });
  }
  const anyApproved = c.pending.some((p) => approved.includes(p.id));
  c.pending = [];
  const reply = anyApproved ? (pendingAnswers.get(id) ?? 'Done.') : "Okay, I didn't create it. You can create it later from Monitors → New Monitor with the same query.";
  await streamText(`\n\n${reply}`, emit, signal);
  answer.text += `\n\n${reply}`;
  c.updatedAt = Date.now();
  emit({ type: 'done', awaiting_approval: false, input_tokens: 900, output_tokens: 80 });
}

export async function report(req: AssistReportRequest): Promise<AssistReport> {
  const rows = usageRows().filter((r) => r.at >= req.range.from && r.at < req.range.to && (!req.surface || r.surface === req.surface) && (!req.userId || r.user === req.userId));
  const bucket = (unit: number, at: number) => Math.floor(at / unit) * unit;
  const group = (unit: number, value: (rs: UsageRow[]) => number) => {
    const by = new Map<number, UsageRow[]>();
    for (const r of rows) by.set(bucket(unit, r.at), [...(by.get(bucket(unit, r.at)) ?? []), r]);
    return [...by.entries()].sort((a, b) => a[0] - b[0]).map(([at, rs]) => ({ at, count: value(rs) }));
  };
  const surfaces = new Map<string, number>();
  for (const r of rows) surfaces.set(r.surface, (surfaces.get(r.surface) ?? 0) + 1);
  return {
    totalUsers: new Set(rows.map((r) => r.user)).size,
    totalMessages: rows.length,
    totalConversations: rows.filter((r) => r.newConversation).length,
    inputTokens: rows.length * 1800,
    outputTokens: rows.length * 240,
    weeklyActiveUsers: group(7 * DAY, (rs) => new Set(rs.map((r) => r.user)).size),
    messagesPerDay: group(DAY, (rs) => rs.length),
    conversationsPerDay: group(DAY, (rs) => rs.filter((r) => r.newConversation).length),
    messagesBySurface: [...surfaces.entries()].sort((a, b) => b[1] - a[1]).map(([surface, count]) => ({ surface, count })),
  };
}

/** Test hook: start over. */
export function resetAssistMock(): void {
  conversations = null;
  usage = null;
  settings = { context: '', disabledSkills: [] };
  pendingAnswers.clear();
}
