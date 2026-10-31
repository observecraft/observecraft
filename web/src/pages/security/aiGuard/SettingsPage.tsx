import { useEffect, useState } from 'react';
import type { AiGuardSettings } from '../../../api/aiGuard/client';
import { ErrorState, Loading } from '../../../components/ui/AsyncState';
import { CategoryChip } from './AiGuardLayout';
import { useAiGuardCategories, useAiGuardSettings, useAiGuardSummary, useUpdateAiGuardSettings } from './queries';

const DAY = 86_400_000;

/** AI Guard → Settings: blocking per service, the confidence threshold, categories, and how to connect the SDK. */
export function AiGuardSettingsPage() {
  const q = useAiGuardSettings();
  const cats = useAiGuardCategories();
  const [now] = useState(() => Date.now());
  const summary = useAiGuardSummary({ range: { from: now - 30 * DAY, to: now }, query: '' });
  const save = useUpdateAiGuardSettings();
  const [s, setS] = useState<AiGuardSettings | null>(null);
  useEffect(() => {
    if (q.data) setS({ ...q.data, blockingServices: q.data.blockingServices ?? [] });
  }, [q.data]);
  if (q.isLoading || !s) return q.isError ? <ErrorState error={q.error} /> : <Loading />;
  const services = [...new Set([...(summary.data?.services ?? []), ...(s.blockingServices ?? [])])].sort();
  const toggleService = (svc: string) =>
    setS({ ...s, blockingServices: (s.blockingServices ?? []).includes(svc) ? (s.blockingServices ?? []).filter((x) => x !== svc) : [...(s.blockingServices ?? []), svc] });
  const toggleCat = (c: string) => setS({ ...s, disabledCategories: s.disabledCategories.includes(c) ? s.disabledCategories.filter((x) => x !== c) : [...s.disabledCategories, c] });
  const dirty = JSON.stringify(s) !== JSON.stringify({ ...q.data, blockingServices: q.data?.blockingServices ?? [] });
  return (
    <div className="sec-page aig-page aig-settings">
      <section className="aig-box">
        <h4>Blocking</h4>
        <p className="muted">AI Guard always evaluates; the SDK blocks an unsafe step only for services in blocking mode (the evaluation's is_blocking_enabled).</p>
        <label className="aig-check">
          <input type="checkbox" checked={s.blockingEnabled} onChange={(e) => setS({ ...s, blockingEnabled: e.target.checked })} /> Block unsafe steps of every service
        </label>
        {!s.blockingEnabled && (
          <table className="mon-table syn-table aig-table">
            <thead>
              <tr>
                <th>SERVICE</th>
                <th>MODE</th>
              </tr>
            </thead>
            <tbody>
              {services.length ? (
                services.map((svc) => (
                  <tr key={svc}>
                    <td>{svc}</td>
                    <td>
                      <label className="aig-check">
                        <input type="checkbox" checked={(s.blockingServices ?? []).includes(svc)} onChange={() => toggleService(svc)} />
                        <span className={`aig-mode${(s.blockingServices ?? []).includes(svc) ? ' blocking' : ''}`}>{(s.blockingServices ?? []).includes(svc) ? 'BLOCKING' : 'MONITORING'}</span>
                      </label>
                    </td>
                  </tr>
                ))
              ) : (
                <tr>
                  <td colSpan={2} className="muted">
                    No service has called AI Guard in the past 30 days.
                  </td>
                </tr>
              )}
            </tbody>
          </table>
        )}
      </section>
      <section className="aig-box">
        <h4>Detection</h4>
        <label className="aig-range">
          Confidence threshold <b>{Math.round(s.threshold * 100)}%</b>
          <input type="range" min={5} max={99} value={Math.round(s.threshold * 100)} onChange={(e) => setS({ ...s, threshold: Number(e.target.value) / 100 })} />
        </label>
        <label className="aig-check">
          <input type="checkbox" checked={s.sdsEnabled} onChange={(e) => setS({ ...s, sdsEnabled: e.target.checked })} /> Scan for sensitive data (PII, credentials, payment data)
        </label>
        <h5>Attack categories</h5>
        <ul className="aig-cat-list">
          {(cats.data ?? []).map((c) => (
            <li key={c.category}>
              <label className="aig-check">
                <input type="checkbox" checked={!s.disabledCategories.includes(c.category)} onChange={() => toggleCat(c.category)} />
                <CategoryChip category={c.category} />
              </label>
              <span className="muted">{c.description}</span>
            </li>
          ))}
        </ul>
      </section>
      <div className="aig-pg-actions">
        {save.isError && <ErrorState error={save.error} />}
        <span className="aig-grow" />
        <button className="btn btn-primary" disabled={!dirty || save.isPending} onClick={() => save.mutate(s)}>
          Save
        </button>
      </div>
      <section className="aig-box">
        <h4>Connect your applications</h4>
        <p className="muted">The tracers' AI Guard SDKs call ObserveCraft's API with an API key and an application key. Point them at this installation:</p>
        <pre className="aig-code">{`DD_AI_GUARD_ENABLED=true
DD_AI_GUARD_ENDPOINT=${window.location.origin}/api/v2/ai-guard
DD_API_KEY=<ingest key>
DD_APP_KEY=<application key>`}</pre>
      </section>
    </div>
  );
}
