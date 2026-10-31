import { useEffect, useMemo, useState } from 'react';
import { Link } from 'react-router-dom';
import { ChevronRight, Pause, Play, Plus, Search, Settings2, Zap } from 'lucide-react';
import type { ApmPoint } from '../../../api/apmSettings/client';
import { ErrorState, Loading } from '../../../components/ui/AsyncState';
import { big as compact } from './format';
import { useApmFlow } from './queries';

const WINDOW_MS = 14 * 24 * 3600_000;

/** Bars of a span count series, with its peak and first/middle dates. */
export function MiniBars({ points, tone }: { points: ApmPoint[]; tone: 'green' | 'purple' | 'pink' }) {
  const max = Math.max(1, ...points.map((p) => p.v));
  const day = (t: number) => new Date(t).toLocaleDateString(undefined, { month: 'short', day: 'numeric' });
  return (
    <div className={`aps-bars aps-bars-${tone}`}>
      <span className="aps-bars-max">{compact(max)}</span>
      <div className="aps-bars-plot">
        {points.map((p) => (
          <i key={p.t} style={{ height: `${Math.max(2, (p.v / max) * 100)}%` }} title={`${new Date(p.t).toLocaleString()}: ${compact(p.v)} spans`} />
        ))}
      </div>
      {points.length > 1 && (
        <div className="aps-bars-axis">
          <span>{day(points[0].t)}</span>
          <span>{day(points[Math.floor(points.length / 2)].t)}</span>
        </div>
      )}
    </div>
  );
}

function pct(part: number, whole: number): string {
  if (whole <= 0) return '0%';
  const p = (part / whole) * 100;
  return `${p >= 99.95 ? 100 : p.toFixed(1)}%`;
}

/** APM → Settings → Overview: how spans go from the tracers to ingestion, processing and indexing. */
export function ApmSettingsOverviewPage() {
  const [paused, setPaused] = useState(false);
  // A fixed past-two-weeks window, refreshed every minute unless paused.
  const [anchor, setAnchor] = useState(() => Math.floor(Date.now() / 60_000) * 60_000);
  const range = useMemo(() => ({ from: anchor - WINDOW_MS, to: anchor }), [anchor]);
  const q = useApmFlow(range);
  useEffect(() => {
    if (paused) return;
    const id = setInterval(() => setAnchor(Math.floor(Date.now() / 60_000) * 60_000), 60_000);
    return () => clearInterval(id);
  }, [paused]);
  const f = q.data;
  return (
    <div className="settings-pad aps-overview">
      <div className="aps-overview-head">
        <h1>How your application spans flow through APM</h1>
        <span className="aps-window">PAST 2 WEEKS</span>
      </div>
      {q.isLoading ? (
        <Loading />
      ) : q.isError || !f ? (
        <ErrorState error={q.error} onRetry={() => q.refetch()} />
      ) : (
        <div className="aps-flow">
          <section className="aps-zone aps-zone-infra">
            <button className="icon-plain aps-pause" onClick={() => setPaused((p) => !p)} title={paused ? 'Resume updates' : 'Pause updates'}>
              {paused ? <Play size={14} /> : <Pause size={14} />}
            </button>
            <span className="aps-zone-label">YOUR INFRASTRUCTURE</span>
            <div className="aps-sources">
              <div className="aps-source">
                <span>SDKs</span>
                <b>{compact(f.sdks)}</b>
              </div>
              <div className="aps-source">
                <span>AGENTS</span>
                <b>{compact(f.agents)}</b>
              </div>
              <div className="aps-source aps-source-otel">
                <span>OTEL SOURCES</span>
                <b>{compact(f.otelSources)}</b>
              </div>
            </div>
            <svg className="aps-wires-top" viewBox="0 0 1000 40" preserveAspectRatio="none" aria-hidden>
              <path d="M 151 0 V 40 M 470 0 V 40" />
            </svg>
            <div className="aps-card aps-card-green">
              <div className="aps-card-title">
                <span>Generated spans</span>
                <b>{compact(f.spansGenerated)}</b>
              </div>
              <MiniBars points={f.generated} tone="green" />
              <div className="aps-card-side">
                <span>From {compact(f.services)} services</span>
                <Link to="/apm/instrument" className="btn btn-sm aps-card-btn">
                  <Plus size={13} /> APM Setup
                </Link>
              </div>
            </div>
            <div className="aps-pill">
              <div>
                <b className="aps-pill-title aps-pill-blue">INGESTION CONTROL</b>
                <span>
                  <b>{pct(f.spansIngested, f.spansGenerated)}</b> spans sampled before ingestion
                </span>
              </div>
              <Link to="/apm/settings/ingestion" className="btn btn-primary btn-sm aps-manage">
                <Settings2 size={13} /> Manage
              </Link>
            </div>
            <svg className="aps-otel-wire" viewBox="0 0 760 100" preserveAspectRatio="none" aria-hidden>
              <path d="M 584 0 C 584 70, 393 72, 393 100" />
            </svg>
            <span className="aps-spine" />
          </section>
          <section className="aps-zone aps-zone-platform">
            <span className="aps-spine" />
            <span className="aps-zone-label">OBSERVECRAFT PLATFORM</span>
            <img src="/logo.svg" alt="" width={30} height={30} className="aps-zone-logo" />
            <div className="aps-processing">
              <span className="aps-processing-title">Processing at ingestion</span>
              <div className="aps-processing-steps">
                <Link to="/apm/settings/pipelines" className="aps-step">
                  <b>{f.pipelines}</b>
                  <span>processing pipelines</span>
                </Link>
                <ChevronRight size={16} className="aps-step-arrow" />
                <span className="aps-step" title="Service remapping rules aren't available yet">
                  <b>{f.serviceRemappings}</b>
                  <span>service remapping</span>
                </span>
                <ChevronRight size={16} className="aps-step-arrow" />
                <Link to="/apm/settings/metrics" className="aps-step">
                  <b>{f.metrics}</b>
                  <span>metrics generated</span>
                </Link>
              </div>
            </div>
            <div className="aps-card aps-card-purple">
              <div className="aps-card-title">
                <span>Ingested spans</span>
                <b>{compact(f.spansIngested)}</b>
              </div>
              <MiniBars points={f.ingested} tone="purple" />
              <div className="aps-card-side">
                <span>Searchable for {f.retentionDays} days</span>
                <Link to="/apm/traces" className="btn btn-sm aps-card-btn">
                  <Zap size={13} /> Live Search
                </Link>
              </div>
            </div>
            <div className="aps-pill">
              <div>
                <b className="aps-pill-title aps-pill-pink">RETENTION FILTERS</b>
                <span>
                  <b>{pct(f.spansIndexed, f.spansIngested)}</b> spans kept for indexed search
                </span>
              </div>
              <Link to="/apm/settings/retention-filters" className="btn btn-primary btn-sm aps-manage">
                <Settings2 size={13} /> Manage
              </Link>
            </div>
            <div className="aps-card aps-card-pink">
              <div className="aps-card-title">
                <span>Indexed spans</span>
                <b>{compact(f.spansIndexed)}</b>
              </div>
              <MiniBars points={f.indexed} tone="pink" />
              <div className="aps-card-side">
                <span>Available for {f.retentionDays} days</span>
                <Link to="/apm/traces" className="btn btn-sm aps-card-btn">
                  <Search size={13} /> Trace Search
                </Link>
              </div>
            </div>
            <p className="muted aps-note">
              Every ingested span stays searchable for the org's span retention; the retention filters show what an indexing policy would keep ({f.retentionFilters} enabled).
            </p>
          </section>
        </div>
      )}
    </div>
  );
}
