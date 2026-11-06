// The error of a log in its side panel: Datadog's error.kind / error.message / error.stack (nested or dotted, from
// the application's JSON, a pipeline, or the server reading a plain-text stack trace: log_stack.rs), else
// OpenTelemetry's exception.type / .message / .stacktrace. Library frames are muted, causes stand out.
import { useState } from 'react';
import type { JsonValue } from '../../api/types';

const SHOWN = 14;

/** Frames of the runtime and frameworks, not the application's own code. */
const LIBRARY =
  /^\s*(?:at |File ")(?:java\.|javax\.|jdk\.|sun\.|kotlin\.|scala\.|org\.springframework\.|org\.apache\.|io\.netty\.|reactor\.|node:internal|internal\/|System\.|Microsoft\.|.*\/(?:site-packages|dist-packages|lib\/python\d|node_modules)\/)/;

function text(v: JsonValue | undefined): string {
  return typeof v === 'string' ? v : v == null ? '' : JSON.stringify(v);
}

/** A log's error attributes, or null when it has none. */
export function errorOf(attributes: Record<string, JsonValue>): { kind: string; message: string; stack: string } | null {
  const nested = (o: JsonValue | undefined, k: string) => (o && typeof o === 'object' && !Array.isArray(o) ? (o as Record<string, JsonValue>)[k] : undefined);
  const get = (group: string, k: string) => text(attributes[`${group}.${k}`] ?? nested(attributes[group], k));
  const kind = get('error', 'kind') || get('error', 'type') || get('exception', 'type');
  const message = get('error', 'message') || get('exception', 'message');
  const stack = get('error', 'stack') || get('exception', 'stacktrace');
  return stack || kind ? { kind, message, stack } : null;
}

export function LogError({ attributes }: { attributes: Record<string, JsonValue> }) {
  const [all, setAll] = useState(false);
  const error = errorOf(attributes);
  if (!error) return null;
  const lines = error.stack ? error.stack.split('\n') : [];
  const shown = all ? lines : lines.slice(0, SHOWN);
  return (
    <section className="detail-section">
      <h4>ERROR</h4>
      <div className="log-error">
        {(error.kind || error.message) && (
          <div className="log-error-head">
            {error.kind && <b>{error.kind}</b>}
            {error.message && <span>{error.message}</span>}
          </div>
        )}
        {lines.length > 0 && (
          <pre className="log-error-stack mono">
            {shown.map((l, i) => (
              <div key={i} className={/^\s*Caused by:|^\S.*(?:Exception|Error)\b/.test(l) && i > 0 ? 'log-error-cause' : LIBRARY.test(l) ? 'log-error-lib' : undefined}>
                {l || ' '}
              </div>
            ))}
          </pre>
        )}
        {lines.length > SHOWN && (
          <button className="link-btn" onClick={() => setAll((a) => !a)}>
            {all ? 'Show less' : `Show all ${lines.length} lines`}
          </button>
        )}
      </div>
    </section>
  );
}
