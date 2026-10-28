import { useEffect, useState } from 'react';
import { keepPreviousData, useQuery } from '@tanstack/react-query';
import { Plus, Trash2, X } from 'lucide-react';
import { parseGrok, PROCESSOR_TYPES, type GrokParser, type Processor, type ProcessorType } from '../../api/pipelines/client';
import { JsonTree } from '../../components/json/JsonTree';
import { Modal } from '../../components/ui/Modal';

/** A new processor of a type, with the fields its form edits. */
export function blankProcessor(type: ProcessorType): Processor {
  switch (type) {
    case 'grok-parser':
      return { type, name: '', enabled: true, source: 'message', samples: [''], grok: { supportRules: '', matchRules: '' } };
    case 'attribute-remapper':
      return { type, name: '', enabled: true, sources: [], sourceType: 'attribute', target: '', targetType: 'attribute', targetFormat: 'auto', preserveSource: false, overrideOnConflict: false };
    case 'category-processor':
      return { type, name: '', enabled: true, target: '', categories: [{ filter: { query: '' }, name: '' }] };
    case 'string-builder-processor':
      return { type, name: '', enabled: true, template: '', target: '', replaceMissing: false };
    case 'lookup-processor':
      return { type, name: '', enabled: true, source: '', target: '', lookupTable: [], defaultLookup: '' };
    case 'arithmetic-processor':
      return { type, name: '', enabled: true, expression: '', target: '', replaceMissing: false };
    case 'url-parser':
      return { type, name: '', enabled: true, sources: ['http.url'], target: 'http.url_details' };
    case 'pipeline':
      return { type, name: '', enabled: true, filter: { query: '' }, processors: [] };
    default:
      return { type, name: '', enabled: true, sources: [] };
  }
}

interface Props {
  /** The processor to edit, or null to pick a type for a new one. */
  processor: Processor | null;
  /** Nested pipelines can't hold nested pipelines. */
  nested: boolean;
  saving: boolean;
  error: Error | null;
  onSave: (p: Processor) => void;
  onClose: () => void;
}

/** Add or edit one processor of a pipeline. */
export function ProcessorEditor({ processor, nested, saving, error, onSave, onClose }: Props) {
  const [draft, setDraft] = useState<Processor | null>(processor ? structuredClone(processor) : null);
  const label = PROCESSOR_TYPES.find((t) => t.type === draft?.type)?.label ?? 'Processor';
  return (
    <Modal onClose={onClose} label={processor ? `Edit ${label}` : 'Add processor'} width={draft?.type === 'grok-parser' ? 960 : 680} height="auto">
      <form
        className="small-modal ar-form pl-form"
        onSubmit={(e) => {
          e.preventDefault();
          if (draft) onSave(draft);
        }}
      >
        <div className="modal-title-row">
          <h3>{processor ? `Edit ${label}` : draft ? `New ${label}` : 'Add Processor'}</h3>
          <button type="button" className="icon-plain" onClick={onClose} title="Close">
            <X size={18} />
          </button>
        </div>
        {!draft ? (
          <div className="pl-type-grid">
            {PROCESSOR_TYPES.filter((t) => !nested || t.type !== 'pipeline').map((t) => (
              <button key={t.type} type="button" className="pl-type" onClick={() => setDraft(blankProcessor(t.type))}>
                <b>{t.label}</b>
                <span className="muted">{t.hint}</span>
              </button>
            ))}
          </div>
        ) : (
          <>
            <div className="pl-name-row">
              <label className="form-label">
                Name
                <input value={draft.name ?? ''} onChange={(e) => setDraft({ ...draft, name: e.target.value })} placeholder={label} maxLength={200} />
              </label>
              <label className="ar-check">
                <input type="checkbox" checked={draft.enabled !== false} onChange={(e) => setDraft({ ...draft, enabled: e.target.checked })} /> Enabled
              </label>
            </div>
            <Fields draft={draft} onChange={setDraft} />
            {error && <p className="form-error pl-errors">{error.message}</p>}
            <div className="small-modal-actions">
              <button type="button" className="btn" onClick={onClose}>
                Cancel
              </button>
              <button className="btn btn-primary" disabled={saving}>
                {processor ? 'Save' : 'Add'}
              </button>
            </div>
          </>
        )}
      </form>
    </Modal>
  );
}

const list = (s: string) =>
  s
    .split(',')
    .map((x) => x.trim())
    .filter(Boolean);

/** A comma-separated attribute list (`level, severity`): the first one present is used. */
function SourcesInput({ value, onChange, label = 'Source attributes (the first one present is used)' }: { value: string[]; onChange: (v: string[]) => void; label?: string }) {
  const [textValue, setText] = useState(value.join(', '));
  return (
    <label className="form-label">
      {label}
      <input className="mono" value={textValue} onChange={(e) => (setText(e.target.value), onChange(list(e.target.value)))} placeholder="level, severity" required />
    </label>
  );
}

function Fields({ draft, onChange }: { draft: Processor; onChange: (p: Processor) => void }) {
  const input = (label: string, value: string, set: (v: string) => void, placeholder = '', mono = true) => (
    <label className="form-label">
      {label}
      <input className={mono ? 'mono' : undefined} value={value} onChange={(e) => set(e.target.value)} placeholder={placeholder} required />
    </label>
  );
  const check = (label: string, value: boolean | undefined, set: (v: boolean) => void) => (
    <label className="ar-check">
      <input type="checkbox" checked={!!value} onChange={(e) => set(e.target.checked)} /> {label}
    </label>
  );
  switch (draft.type) {
    case 'grok-parser':
      return <GrokFields draft={draft} onChange={onChange} />;
    case 'status-remapper':
    case 'date-remapper':
    case 'service-remapper':
    case 'message-remapper':
    case 'trace-id-remapper':
      return (
        <>
          <p className="muted pl-help">
            {draft.type === 'status-remapper' && 'Maps words (error, warning, notice, debug, …) and syslog severities 0–7 onto error, warn, info and debug.'}
            {draft.type === 'date-remapper' && 'Accepts epoch seconds or milliseconds, ISO 8601 and common log date formats.'}
            {draft.type === 'service-remapper' && 'Sets the log’s service.'}
            {draft.type === 'message-remapper' && 'Sets the log’s message (shown in the explorer and searched as free text).'}
            {draft.type === 'trace-id-remapper' && 'Links the log to its trace; decimal (Datadog) and hex ids are accepted.'}
          </p>
          <SourcesInput value={draft.sources} onChange={(sources) => onChange({ ...draft, sources })} />
        </>
      );
    case 'attribute-remapper':
      return (
        <>
          <div className="ar-grid">
            <label className="form-label">
              Source type
              <select value={draft.sourceType ?? 'attribute'} onChange={(e) => onChange({ ...draft, sourceType: e.target.value as 'attribute' | 'tag' })}>
                <option value="attribute">Attribute</option>
                <option value="tag">Tag</option>
              </select>
            </label>
            <label className="form-label">
              Target type
              <select value={draft.targetType ?? 'attribute'} onChange={(e) => onChange({ ...draft, targetType: e.target.value as 'attribute' | 'tag' })}>
                <option value="attribute">Attribute</option>
                <option value="tag">Tag</option>
              </select>
            </label>
            <label className="form-label">
              Target format
              <select value={draft.targetFormat ?? 'auto'} onChange={(e) => onChange({ ...draft, targetFormat: e.target.value as 'auto' })}>
                <option value="auto">Keep as is</option>
                <option value="string">String</option>
                <option value="integer">Integer</option>
                <option value="double">Double</option>
              </select>
            </label>
          </div>
          <SourcesInput value={draft.sources} onChange={(sources) => onChange({ ...draft, sources })} label="Sources (the first one present is used)" />
          {input('Target', draft.target, (target) => onChange({ ...draft, target }), 'network.client.ip')}
          {check('Keep the source', draft.preserveSource, (preserveSource) => onChange({ ...draft, preserveSource }))}
          {check('Replace the target when it already exists', draft.overrideOnConflict, (overrideOnConflict) => onChange({ ...draft, overrideOnConflict }))}
        </>
      );
    case 'category-processor': {
      const setRule = (i: number, r: Partial<{ query: string; name: string }>) =>
        onChange({ ...draft, categories: draft.categories.map((c, j) => (j === i ? { filter: { query: r.query ?? c.filter.query }, name: r.name ?? c.name } : c)) });
      return (
        <>
          {input('Target attribute', draft.target, (target) => onChange({ ...draft, target }), 'http.status_category')}
          <div className="ix-excl-head">
            <b>Categories</b>
            <span className="muted">The first category whose search matches sets the target.</span>
          </div>
          {draft.categories.map((c, i) => (
            <div key={i} className="ix-excl-row">
              <input className="mono ix-excl-query" value={c.filter.query} onChange={(e) => setRule(i, { query: e.target.value })} placeholder="@http.status_code:[500 TO 599]" required />
              <input className="ix-excl-name" value={c.name} onChange={(e) => setRule(i, { name: e.target.value })} placeholder="error" required />
              <button type="button" className="icon-plain" title="Remove" onClick={() => onChange({ ...draft, categories: draft.categories.filter((_, j) => j !== i) })}>
                <Trash2 size={15} />
              </button>
            </div>
          ))}
          <button type="button" className="btn btn-sm ix-add-excl" onClick={() => onChange({ ...draft, categories: [...draft.categories, { filter: { query: '' }, name: '' }] })}>
            <Plus size={13} /> Add Category
          </button>
        </>
      );
    }
    case 'string-builder-processor':
      return (
        <>
          {input('Template — %{attribute} inserts an attribute', draft.template, (template) => onChange({ ...draft, template }), '%{http.method} %{http.url_details.path}')}
          {input('Target attribute', draft.target, (target) => onChange({ ...draft, target }), 'http.route')}
          {check('Use an empty value for missing attributes (otherwise the target is not set)', draft.replaceMissing, (replaceMissing) => onChange({ ...draft, replaceMissing }))}
        </>
      );
    case 'lookup-processor':
      return (
        <>
          <div className="ar-grid">
            {input('Source attribute', draft.source, (source) => onChange({ ...draft, source }), 'warehouse')}
            {input('Target attribute', draft.target, (target) => onChange({ ...draft, target }), 'warehouse_name')}
          </div>
          <label className="form-label">
            Lookup table — one key,value per line
            <textarea
              className="mono"
              rows={6}
              value={draft.lookupTable.join('\n')}
              onChange={(e) => onChange({ ...draft, lookupTable: e.target.value.split('\n') })}
              placeholder={'fra1,Frankfurt\niad2,Ashburn'}
            />
          </label>
          <label className="form-label">
            Default value (when the key isn’t in the table)
            <input value={draft.defaultLookup ?? ''} onChange={(e) => onChange({ ...draft, defaultLookup: e.target.value })} placeholder="none: leave the target unset" />
          </label>
        </>
      );
    case 'arithmetic-processor':
      return (
        <>
          {input('Expression — attributes, numbers, + - * / and parentheses', draft.expression, (expression) => onChange({ ...draft, expression }), '(duration - queue_time) * 1000')}
          {input('Target attribute', draft.target, (target) => onChange({ ...draft, target }), 'work_ms')}
          {check('Count missing attributes as 0 (otherwise the target is not set)', draft.replaceMissing, (replaceMissing) => onChange({ ...draft, replaceMissing }))}
        </>
      );
    case 'url-parser':
      return (
        <>
          <SourcesInput value={draft.sources} onChange={(sources) => onChange({ ...draft, sources })} label="URL attribute" />
          {input('Target attribute (gets scheme, host, port, path and queryString)', draft.target, (target) => onChange({ ...draft, target }), 'http.url_details')}
        </>
      );
    case 'pipeline':
      return (
        <>
          {input('Filter — the nested processors run on matching logs only', draft.filter.query, (query) => onChange({ ...draft, filter: { query } }), 'status:error', true)}
          <p className="muted pl-help">Add processors to the nested pipeline from the pipeline list once it is saved.</p>
        </>
      );
  }
}

function useDebounced<T>(value: T, ms: number): T {
  const [v, setV] = useState(value);
  useEffect(() => {
    const t = setTimeout(() => setV(value), ms);
    return () => clearTimeout(t);
  }, [value, ms]);
  return v;
}

/** Grok parser: samples, rules and each sample's parse, live. */
function GrokFields({ draft, onChange }: { draft: GrokParser; onChange: (p: Processor) => void }) {
  const samples = draft.samples?.length ? draft.samples : [''];
  const grok = { supportRules: draft.grok.supportRules ?? '', matchRules: draft.grok.matchRules ?? '' };
  const [showSupport, setShowSupport] = useState(!!grok.supportRules.trim());
  const req = useDebounced({ ...grok, samples: samples.filter((s) => s.trim()) }, 300);
  const preview = useQuery({ queryKey: ['grok-preview', req], queryFn: ({ signal }) => parseGrok(req, signal), enabled: !!req.matchRules.trim(), placeholderData: keepPreviousData });
  const setSample = (i: number, v: string) => onChange({ ...draft, samples: samples.map((s, j) => (j === i ? v : s)) });
  const setRules = (p: Partial<typeof grok>) => onChange({ ...draft, grok: { ...grok, ...p } });
  // Results line up with the non-empty samples sent.
  const sent = samples.map((s) => s.trim() !== '');
  const resultFor = (i: number) => (sent[i] && preview.data ? preview.data.samples[sent.slice(0, i).filter(Boolean).length] : undefined);
  return (
    <>
      <label className="form-label">
        Attribute to parse
        <input className="mono" value={draft.source ?? 'message'} onChange={(e) => onChange({ ...draft, source: e.target.value })} placeholder="message" />
      </label>
      <div className="ix-excl-head">
        <b>Log samples</b>
        <span className="muted">Paste up to five lines this parser should handle; each shows what the rules extract.</span>
      </div>
      {samples.map((s, i) => {
        const r = resultFor(i);
        return (
          <div key={i} className="pl-sample">
            <div className="ix-excl-row">
              <textarea className="mono pl-sample-text" rows={2} value={s} onChange={(e) => setSample(i, e.target.value)} placeholder="10.0.4.21 GET /api/cart 200" />
              <button type="button" className="icon-plain" title="Remove sample" disabled={samples.length === 1} onClick={() => onChange({ ...draft, samples: samples.filter((_, j) => j !== i) })}>
                <Trash2 size={15} />
              </button>
            </div>
            {r && (
              <div className={`pl-sample-result${r.matched ? ' ok' : ' miss'}`}>
                {r.matched ? (
                  <>
                    <span className="pl-sample-rule">
                      Matched rule <code>{r.rule}</code>
                    </span>
                    {Object.keys(r.attributes).length ? <JsonTree data={r.attributes} expandDepth={3} /> : <span className="muted">nothing extracted</span>}
                  </>
                ) : (
                  <span>No rule matches this sample: the log stays as it is.</span>
                )}
              </div>
            )}
          </div>
        );
      })}
      {samples.length < 5 && (
        <button type="button" className="btn btn-sm ix-add-excl" onClick={() => onChange({ ...draft, samples: [...samples, ''] })}>
          <Plus size={13} /> Add Sample
        </button>
      )}
      <label className="form-label">
        Match rules — one per line: <span className="mono">name pattern</span>; the first rule matching the whole value wins
        <textarea
          className="mono"
          rows={5}
          value={grok.matchRules}
          onChange={(e) => setRules({ matchRules: e.target.value })}
          placeholder={'access %{ip:network.client.ip} %{word:http.method} %{notSpace:http.url} %{integer:http.status_code}'}
          required
        />
      </label>
      <p className="muted pl-help">
        <code>
          %{'{'}matcher:attribute:filter{'}'}
        </code>
        . Matchers: word, notSpace, data, integer, number, numberStr, boolean, ip, ipv4, ipv6, hostname, ipOrHost, port, quotedString, uuid, mac, date(&quot;dd/MMM/yyyy:HH:mm:ss Z&quot;),
        regex(&quot;…&quot;). Filters: number, integer, boolean, nullIf(&quot;-&quot;), lowercase, uppercase, scale(n), array(&quot;,&quot;), json, keyvalue, querystring, decodeuricomponent. Dots in
        attribute names nest them (<code>http.status_code</code>).
      </p>
      {showSupport ? (
        <label className="form-label">
          Support rules — reusable patterns, included with{' '}
          <span className="mono">
            %{'{'}name{'}'}
          </span>
          <textarea className="mono" rows={4} value={grok.supportRules} onChange={(e) => setRules({ supportRules: e.target.value })} placeholder={'_method %{word:http.method}'} />
        </label>
      ) : (
        <button type="button" className="btn btn-sm ix-add-excl" onClick={() => setShowSupport(true)}>
          <Plus size={13} /> Support Rules
        </button>
      )}
      {preview.data?.error && <p className="form-error">{preview.data.error}</p>}
      {preview.error && <p className="form-error">{(preview.error as Error).message}</p>}
    </>
  );
}
