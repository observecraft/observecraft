// In-browser log pipelines for mock mode (Logs → Configuration → Pipelines). State lives in memory
// for the session; validation follows the server (routes/log_pipelines.rs) and the test endpoints
// run the TypeScript engine (pipelineEngine.ts).
import { INTEGRATION_PIPELINES, type IntegrationPipelineDoc } from '../pipelines/library';
import type { GrokTestRequest, GrokTestResult, LibraryPipeline, LogPipeline, LogPipelineInput, PipelineTestRequest, PipelineTestResult, Processor } from '../pipelines/types';
import { parseGrok as runGrok, processorErrors, runPipeline } from './pipelineEngine';

const MAX_PIPELINES = 100;
const MAX_PROCESSORS = 50;

const fail = (message: string): never => {
  throw new Error(message);
};
const copy = <T>(v: T): T => structuredClone(v);

interface State {
  pipelines: LogPipeline[];
  seq: number;
}

let state: State | null = null;

/** Demo pipelines for the e-commerce services, plus the nginx integration pipeline. */
function seed(): LogPipeline[] {
  const custom = (id: string, name: string, filter: string, processors: Processor[], enabled = true): Omit<LogPipeline, 'position'> => ({
    id,
    name,
    filter,
    processors,
    enabled,
    source: 'custom',
    readOnly: false,
  });
  const nginx = libraryPipeline('nginx');
  const pipelines: Omit<LogPipeline, 'position'>[] = [
    custom('pl-0001', 'api-gateway access logs', 'service:api-gateway source:nginx', [
      {
        type: 'grok-parser',
        name: 'Parse access lines',
        source: 'message',
        samples: ['10.0.4.21 - - [25/Oct/2026:12:59:58 +0000] "GET /api/cart?id=7 HTTP/1.1" 200 512 0.012', '10.0.4.22 - - [25/Oct/2026:13:00:02 +0000] "POST /api/checkout HTTP/1.1" 502 87 1.204'],
        grok: {
          supportRules: '_date %{date("dd/MMM/yyyy:HH:mm:ss Z"):date_access}\n_request %{word:http.method} %{notSpace:http.url} HTTP/%{regex("\\\\d+\\\\.\\\\d+"):http.version}',
          matchRules:
            'access %{ip:network.client.ip} - %{notSpace:http.auth:nullIf("-")} \\[%{_date}\\] "%{_request}" %{integer:http.status_code} %{integer:network.bytes_written} %{number:duration:scale(1000000000)}',
        },
      },
      { type: 'url-parser', name: 'Split the URL', sources: ['http.url'], target: 'http.url_details' },
      { type: 'date-remapper', name: 'Request time is the log date', sources: ['date_access'] },
      {
        type: 'category-processor',
        name: 'Status category',
        target: 'http.status_category',
        categories: [
          { filter: { query: '@http.status_code:[200 TO 399]' }, name: 'OK' },
          { filter: { query: '@http.status_code:[400 TO 499]' }, name: 'warning' },
          { filter: { query: '@http.status_code:[500 TO 599]' }, name: 'error' },
        ],
      },
      { type: 'status-remapper', name: 'Status from the category', sources: ['http.status_category'] },
    ]),
    custom('pl-0002', 'order-service JSON logs', 'service:order-service', [
      { type: 'status-remapper', name: 'Level is the status', sources: ['level', 'severity'] },
      { type: 'message-remapper', name: 'msg is the message', sources: ['msg'] },
      {
        type: 'attribute-remapper',
        name: 'order_id → order.id',
        sources: ['order_id'],
        sourceType: 'attribute',
        target: 'order.id',
        targetType: 'attribute',
        preserveSource: false,
        overrideOnConflict: false,
      },
      { type: 'trace-id-remapper', name: 'Link to traces', sources: ['dd.trace_id'] },
    ]),
    custom('pl-0003', 'billing payment events', 'service:billing', [
      {
        type: 'grok-parser',
        name: 'Parse charge results',
        source: 'message',
        samples: ['charge declined order=ord-81723 amount=129.90 currency=USD reason=insufficient_funds', 'charge approved order=ord-81724 amount=42.00 currency=EUR'],
        grok: { supportRules: '', matchRules: 'charge charge %{word:payment.result} order=%{notSpace:order.id} amount=%{number:payment.amount} %{data::keyvalue}' },
      },
      { type: 'arithmetic-processor', name: 'Amount in cents', expression: 'payment.amount * 100', target: 'payment.amount_cents', replaceMissing: false },
      { type: 'category-processor', name: 'Declines are warnings', target: 'level', categories: [{ filter: { query: '@payment.result:declined' }, name: 'warn' }] },
      { type: 'status-remapper', name: 'Status from level', sources: ['level'] },
    ]),
    custom(
      'pl-0004',
      'Warehouse names',
      'service:(catalog-service OR inventory-service)',
      [
        {
          type: 'lookup-processor',
          name: 'Warehouse code → name',
          source: 'warehouse',
          target: 'warehouse_name',
          lookupTable: ['fra1,Frankfurt', 'iad2,Ashburn', 'sin1,Singapore'],
          defaultLookup: 'Unknown',
        },
        { type: 'string-builder-processor', name: 'Stock summary', template: '%{sku} @ %{warehouse_name}', target: 'stock.summary', replaceMissing: false },
      ],
      false,
    ),
    { id: 'pl-0005', name: nginx.name, filter: nginx.filter, processors: nginx.processors, enabled: true, source: 'integration:nginx', readOnly: true },
  ];
  return pipelines.map((p, position) => ({ ...p, position }));
}

function db(): State {
  state ??= { pipelines: seed(), seq: 6 };
  return state;
}

const ordered = () => [...db().pipelines].sort((a, b) => a.position - b.position);
const find = (id: string) => db().pipelines.find((p) => p.id === id) ?? fail('Pipeline not found');
const nextPosition = () => db().pipelines.reduce((m, p) => Math.max(m, p.position), -1) + 1;
const newId = () => `pl-${String(db().seq++).padStart(4, '0')}`;

function validate(input: LogPipelineInput): LogPipelineInput {
  const name = input.name.trim();
  const filter = (input.filter ?? '').trim();
  if (!name || [...name].length > 100) fail('Pipeline names are 1–100 characters');
  const processors = input.processors ?? [];
  if (processors.length > MAX_PROCESSORS) fail(`At most ${MAX_PROCESSORS} processors per pipeline`);
  const untyped = processors.findIndex((p) => !p?.type);
  if (untyped >= 0) fail(`Processor ${untyped + 1} needs a type`);
  const errors = processorErrors(filter, processors);
  if (errors.length) fail(errors.join('\n'));
  return { name, filter, enabled: input.enabled ?? true, processors: copy(processors) };
}

export function libraryPipeline(id: string): IntegrationPipelineDoc {
  return INTEGRATION_PIPELINES.find((p) => p.id === id) ?? fail('No integration pipeline with that name');
}

export async function listPipelines(): Promise<LogPipeline[]> {
  return copy(ordered());
}

export async function createPipeline(input: LogPipelineInput): Promise<LogPipeline> {
  const fields = validate(input);
  if (db().pipelines.length >= MAX_PIPELINES) fail(`An organization can have at most ${MAX_PIPELINES} pipelines`);
  const p: LogPipeline = { id: newId(), ...fields, position: nextPosition(), source: 'custom', readOnly: false };
  db().pipelines.push(p);
  return copy(p);
}

export async function updatePipeline(id: string, input: LogPipelineInput): Promise<LogPipeline> {
  const fields = validate(input);
  const p = find(id);
  if (p.readOnly) fail('Integration pipelines are read-only: clone it to make changes');
  Object.assign(p, fields);
  return copy(p);
}

export async function setPipelineEnabled(id: string, enabled: boolean): Promise<LogPipeline> {
  const p = find(id);
  p.enabled = enabled;
  return copy(p);
}

export async function deletePipeline(id: string): Promise<void> {
  find(id);
  db().pipelines = db().pipelines.filter((p) => p.id !== id);
}

export async function reorderPipelines(ids: string[]): Promise<void> {
  ids.forEach((id, position) => {
    const p = db().pipelines.find((x) => x.id === id);
    if (p) p.position = position;
  });
}

export async function listLibrary(): Promise<LibraryPipeline[]> {
  const installed = new Set(db().pipelines.map((p) => p.source));
  return INTEGRATION_PIPELINES.map((p) => ({ id: p.id, name: p.name, filter: p.filter, processors: copy(p.processors), upstream: p.source, installed: installed.has(`integration:${p.id}`) }));
}

export async function installLibraryPipeline(id: string): Promise<LogPipeline> {
  const lib = libraryPipeline(id);
  const source = `integration:${lib.id}`;
  if (db().pipelines.some((p) => p.source === source)) fail(`The ${lib.name} pipeline is already installed`);
  const p: LogPipeline = { id: newId(), name: lib.name, filter: lib.filter, processors: copy(lib.processors), enabled: true, position: nextPosition(), source, readOnly: true };
  db().pipelines.push(p);
  return copy(p);
}

export function testPipeline(req: PipelineTestRequest): PipelineTestResult {
  return runPipeline(req.filter ?? '', req.processors ?? [], req.log);
}

export function parseGrok(req: GrokTestRequest): GrokTestResult {
  return runGrok(req);
}
