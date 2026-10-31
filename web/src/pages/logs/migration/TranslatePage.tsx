import { useRef, useState } from 'react';
import { Link, useNavigate, useSearchParams } from 'react-router-dom';
import { useMutation } from '@tanstack/react-query';
import { AlertTriangle, ArrowRight, CheckCircle2, Code2, Copy, FileUp, X } from 'lucide-react';
import { translateSpl, type SplAggregation, type SplTranslation } from '../../../api/logMigration/client';
import './migration.css';

const EXAMPLES: [string, string][] = [
  ['Filter', 'index=web sourcetype=nginx status=500 NOT host=web-test*'],
  ['Aggregation', 'index=web service=checkout | stats count, avg(duration) by host'],
  ['Time series', 'index=web log_level=ERROR | timechart span=5m count by service'],
];

const FUNCTION_LABEL: Record<string, string> = { cardinality: 'unique count', pc50: 'median', pc75: 'p75', pc90: 'p90', pc95: 'p95', pc99: 'p99' };

export function describeAggregation(a: SplAggregation): string {
  const m = a.measures.map((x) => `${FUNCTION_LABEL[x.function] ?? x.function}${x.field ? `(${x.field})` : ''}`).join(', ');
  const parts = [m];
  if (a.groupBy.length) parts.push(`by ${a.groupBy.join(', ')}`);
  if (a.interval && a.interval !== 'auto') parts.push(`every ${a.interval}`);
  if (a.limit) parts.push(`${a.ascending ? 'bottom' : 'top'} ${a.limit}`);
  if (a.condition) parts.push(`where ${a.condition.measure} ${a.condition.op} ${a.condition.value}`);
  return parts.join(' ');
}

const VIZ_LABEL: Record<string, string> = { timeseries: 'Timeseries', toplist: 'Top List', table: 'Table', value: 'Query Value' };

/** The Log Explorer with the translated search (and its aggregation's visualization). */
export function explorerLink(t: SplTranslation): string {
  const p = new URLSearchParams({ query: t.query });
  const viz = t.aggregation?.viz;
  if (viz && viz !== 'value') p.set('viz', viz);
  return `/logs?${p.toString()}`;
}

/** Logs → Configuration → Migration → Translate: one Splunk search at a time, explained. */
export function TranslatePage() {
  const [params] = useSearchParams();
  const navigate = useNavigate();
  const [spl, setSpl] = useState(() => params.get('q') ?? '');
  const [macros, setMacros] = useState<{ name: string; text: string } | null>(null);
  const [copied, setCopied] = useState(false);
  const fileRef = useRef<HTMLInputElement>(null);
  const run = useMutation({ mutationFn: () => translateSpl(spl, macros?.text) });
  const t = run.data;
  const lines = Math.max(4, spl.split('\n').length);

  return (
    <div className="settings-pad mig-page">
      <h1 className="settings-title">Quick Translation</h1>
      <p className="muted">
        See how a Splunk search reads in ObserveCraft, step by step. Moving many searches, alerts and dashboards? Use <Link to="/logs/import">Imports</Link>.
      </p>
      <div className="mig-source-row">
        <span>
          Source: <b>Splunk Processing Language (SPL)</b>
        </span>
        <span className="mig-other" title="SPL is the only source language for now">
          Using another source?
        </span>
      </div>
      <div className="mig-editor">
        <div className="mig-gutter" aria-hidden>
          {Array.from({ length: lines }, (_, i) => (
            <span key={i}>{i + 1}</span>
          ))}
        </div>
        <textarea
          aria-label="SPL query"
          spellCheck={false}
          rows={lines}
          placeholder="Start writing your query"
          value={spl}
          onChange={(e) => setSpl(e.target.value)}
          onKeyDown={(e) => {
            if ((e.metaKey || e.ctrlKey) && e.key === 'Enter' && spl.trim()) run.mutate();
          }}
        />
        <button className="btn mig-editor-btn" title="Load a macros file (CSV)" onClick={() => fileRef.current?.click()}>
          <Code2 size={15} />
        </button>
        <input
          ref={fileRef}
          type="file"
          accept=".csv,text/csv"
          hidden
          onChange={async (e) => {
            const f = e.target.files?.[0];
            if (f) setMacros({ name: f.name, text: await f.text() });
            e.target.value = '';
          }}
        />
      </div>
      <div className="mig-examples">
        <span className="muted">No query handy? Try an example:</span>
        {EXAMPLES.map(([label, q]) => (
          <button key={label} className="btn btn-sm" onClick={() => setSpl(q)}>
            {label}
          </button>
        ))}
        {macros && (
          <span className="pill-chip">
            <FileUp size={12} /> {macros.name}
            <button className="link-muted" aria-label="Remove the macros file" onClick={() => setMacros(null)}>
              <X size={12} />
            </button>
          </span>
        )}
        <button className="btn btn-primary mig-translate" disabled={!spl.trim() || run.isPending} onClick={() => run.mutate()}>
          {run.isPending ? 'Translating…' : 'Translate'}
        </button>
      </div>
      {run.isError && <p className="form-error">{(run.error as Error).message}</p>}
      {t && (
        <section className="mig-result">
          <div className="mig-result-head">
            {t.complete ? <CheckCircle2 size={18} className="mig-ok" /> : <AlertTriangle size={18} className="mig-warn" />}
            <b>{t.complete ? 'Fully translated' : `Translated with ${t.warnings.length} ${t.warnings.length === 1 ? 'warning' : 'warnings'}`}</b>
          </div>
          <div className="mig-out">
            <label>ObserveCraft search</label>
            <div className="mig-code">
              <code>{t.query || '*'}</code>
              <button
                className="btn btn-sm"
                onClick={() => {
                  void navigator.clipboard?.writeText(t.query);
                  setCopied(true);
                  setTimeout(() => setCopied(false), 1500);
                }}
              >
                <Copy size={13} /> {copied ? 'Copied' : 'Copy'}
              </button>
            </div>
            {t.aggregation && (
              <>
                <label>Aggregation</label>
                <div className="mig-agg">
                  <span className="pill-chip">{VIZ_LABEL[t.aggregation.viz] ?? t.aggregation.viz}</span>
                  <span>{describeAggregation(t.aggregation)}</span>
                </div>
              </>
            )}
            {t.earliest && (
              <>
                <label>Time range</label>
                <div className="muted">
                  From <code>{t.earliest}</code>: pick it in the time picker.
                </div>
              </>
            )}
            {t.columns.length > 0 && (
              <>
                <label>Columns</label>
                <div className="chip-row">
                  {t.columns.map((c) => (
                    <span key={c} className="pill-chip mono">
                      {c}
                    </span>
                  ))}
                </div>
              </>
            )}
          </div>
          {t.warnings.length > 0 && (
            <ul className="mig-warnings">
              {t.warnings.map((w) => (
                <li key={w}>
                  <AlertTriangle size={14} /> {w}
                </li>
              ))}
            </ul>
          )}
          <table className="settings-table mig-steps">
            <thead>
              <tr>
                <th>SPL</th>
                <th />
                <th>OBSERVECRAFT</th>
                <th>NOTE</th>
              </tr>
            </thead>
            <tbody>
              {t.explanation.map((s, i) => (
                <tr key={i} className={s.result ? '' : 'row-muted'}>
                  <td className="mono">{s.spl}</td>
                  <td>
                    <ArrowRight size={13} className="muted" />
                  </td>
                  <td className="mono">{s.result || '—'}</td>
                  <td className="muted">{s.note}</td>
                </tr>
              ))}
            </tbody>
          </table>
        </section>
      )}
      <footer className="mig-footer">
        <button
          className="btn"
          onClick={() => {
            setSpl('');
            run.reset();
          }}
        >
          Cancel
        </button>
        <button className="btn btn-primary" disabled={!t} onClick={() => t && navigate(explorerLink(t))}>
          View in Log Explorer
        </button>
      </footer>
    </div>
  );
}
