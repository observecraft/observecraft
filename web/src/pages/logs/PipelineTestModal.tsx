import { useEffect, useState } from 'react';
import { keepPreviousData, useQuery } from '@tanstack/react-query';
import { X } from 'lucide-react';
import { testPipeline, type LogPipeline, type SampleLog } from '../../api/pipelines/client';
import type { JsonValue } from '../../api/types';
import { JsonTree } from '../../components/json/JsonTree';
import { Modal } from '../../components/ui/Modal';

/** A sample log to start from, guessed from the pipeline's filter (`service:x source:y`). */
function initialSample(p: LogPipeline): SampleLog {
  const pick = (key: string) => new RegExp(`(?:^|\\s)${key}:\\(?([\\w.-]+)`).exec(p.filter)?.[1] ?? '';
  const grok = p.processors.find((x) => x.type === 'grok-parser');
  const message = (grok?.type === 'grok-parser' && grok.samples?.find((s) => s.trim())) || '';
  return {
    message,
    status: 'info',
    service: pick('service') || 'storefront-api',
    host: 'i-0a1b2c3d4e5f60001',
    source: pick('source') || pick('service') || 'storefront-api',
    tags: {},
    attributes: {},
    date: null,
    traceId: '',
  };
}

/** Runs a pipeline on a log typed in by the user and shows the processed log. */
export function PipelineTestModal({ pipeline, onClose }: { pipeline: LogPipeline; onClose: () => void }) {
  const [log, setLog] = useState(() => initialSample(pipeline));
  const [attrs, setAttrs] = useState('{}');
  const [debounced, setDebounced] = useState({ log, attrs });
  useEffect(() => {
    const t = setTimeout(() => setDebounced({ log, attrs }), 300);
    return () => clearTimeout(t);
  }, [log, attrs]);
  let attributes: Record<string, JsonValue> | null = null;
  try {
    const parsed = JSON.parse(debounced.attrs || '{}') as unknown;
    if (parsed && typeof parsed === 'object' && !Array.isArray(parsed)) attributes = parsed as Record<string, JsonValue>;
  } catch {
    attributes = null;
  }
  const req = { filter: pipeline.filter, processors: pipeline.processors, log: { ...debounced.log, attributes: attributes ?? {} } };
  const result = useQuery({ queryKey: ['pipeline-test', pipeline.id, req], queryFn: ({ signal }) => testPipeline(req, signal), enabled: attributes !== null, placeholderData: keepPreviousData });
  const set = (p: Partial<SampleLog>) => setLog((l) => ({ ...l, ...p }));
  const out = result.data;
  return (
    <Modal onClose={onClose} label={`Test ${pipeline.name}`} width={900} height="auto">
      <div className="small-modal ar-form pl-form">
        <div className="modal-title-row">
          <h3>Test {pipeline.name}</h3>
          <button type="button" className="icon-plain" onClick={onClose} title="Close">
            <X size={18} />
          </button>
        </div>
        <p className="muted pl-help">
          Runs the saved pipeline on this log, as the indexer does before routing it to an index. Filter: <code>{pipeline.filter || 'all logs'}</code>
        </p>
        <label className="form-label">
          Message
          <textarea className="mono" rows={3} value={log.message} onChange={(e) => set({ message: e.target.value })} />
        </label>
        <div className="ar-grid">
          {(['service', 'source', 'host', 'status'] as const).map((k) => (
            <label key={k} className="form-label">
              {k[0].toUpperCase() + k.slice(1)}
              <input className="mono" value={log[k]} onChange={(e) => set({ [k]: e.target.value })} />
            </label>
          ))}
        </div>
        <label className="form-label">
          Attributes (JSON)
          <textarea className="mono" rows={3} value={attrs} onChange={(e) => setAttrs(e.target.value)} />
        </label>
        {attributes === null && <p className="form-error">The attributes must be a JSON object.</p>}
        {out && (
          <section className={`pl-test-result${out.matched ? ' ok' : ' miss'}`}>
            {!out.matched ? (
              <p>The pipeline’s filter doesn’t match this log: it passes through unchanged.</p>
            ) : (
              <>
                <dl className="pl-test-fields">
                  {(
                    [
                      ['Status', out.log.status],
                      ['Service', out.log.service],
                      ['Date', out.log.date ?? 'unchanged'],
                      ['Message', out.log.message],
                    ] as const
                  ).map(([k, v]) => (
                    <div key={k}>
                      <dt>{k}</dt>
                      <dd className="mono">{v || '—'}</dd>
                    </div>
                  ))}
                  {out.log.traceId && (
                    <div>
                      <dt>Trace ID</dt>
                      <dd className="mono">{out.log.traceId}</dd>
                    </div>
                  )}
                </dl>
                {Object.keys(out.log.attributes).length ? <JsonTree data={out.log.attributes} expandDepth={3} /> : <p className="muted">No attributes.</p>}
              </>
            )}
            {out.errors.map((e) => (
              <p key={e} className="form-error">
                {e}
              </p>
            ))}
          </section>
        )}
        {result.error && <p className="form-error">{(result.error as Error).message}</p>}
      </div>
    </Modal>
  );
}
