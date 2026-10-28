import { useMemo, type ReactNode } from 'react';
import { useSearchParams } from 'react-router-dom';
import { ArrowRight, ChevronDown } from 'lucide-react';
import { Select } from '../../components/ui/Dropdown';
import { Empty, ErrorState, Loading } from '../../components/ui/AsyncState';
import { fmtMs, fmtRate, nodeName } from './format';
import { LineChart } from './LineChart';
import { PathwayChain } from './PipelinesPage';
import { useStreamMap, useStreamMeasure } from './queries';

const picked = (label: ReactNode, placeholder: string) => (
  <>
    <span className={`dd-trigger-label${label ? '' : ' muted'}`}>{label ?? placeholder}</span>
    <ChevronDown size={14} className="dd-caret" />
  </>
);

/** Data Streams → Measure: latency between two services over time, along the hops between them. */
export function MeasurePage() {
  const map = useStreamMap();
  const [params, setParams] = useSearchParams();
  const { starts, ends } = useMemo(() => {
    const edges = map.data?.edges ?? [];
    return {
      starts: [...new Set(edges.filter((e) => e.kind === 'produce').map((e) => e.source))].sort(),
      ends: [...new Set(edges.filter((e) => e.kind === 'consume').map((e) => e.target))].sort(),
    };
  }, [map.data]);
  const first = map.data?.pathways[0];
  const start = params.get('start') ?? first?.nodes[0] ?? null;
  const end = params.get('end') ?? (first && start === first.nodes[0] ? first.nodes[first.nodes.length - 1] : null);
  const q = useStreamMeasure(start, end);
  const set = (k: 'start' | 'end', v: string) =>
    setParams(
      (p) => {
        const n = new URLSearchParams(p);
        n.set(k, v);
        if (k === 'start' && !n.get('end') && end) n.set('end', end);
        if (k === 'end' && !n.get('start') && start) n.set('start', start);
        return n;
      },
      { replace: true },
    );
  if (!map.data) return null;
  const opts = (ids: string[]) => ids.map((id) => ({ value: id, label: nodeName(id) }));
  const r = q.data;
  return (
    <div className="dsm-page">
      <div className="dsm-measure-pick">
        <Select label="Start" value={start ?? ''} options={opts(starts)} onChange={(v) => set('start', v)} trigger={(o) => picked(o?.label, 'Pick a producer')} />
        <ArrowRight size={16} className="muted" />
        <Select label="End" value={end ?? ''} options={opts(ends)} onChange={(v) => set('end', v)} trigger={(o) => picked(o?.label, 'Pick a consumer')} />
        <span className="muted">Time from the start service publishing a message to the end service consuming what follows from it.</span>
      </div>
      {!start || !end ? (
        <Empty>Pick the service messages leave and the service they reach.</Empty>
      ) : start === end ? (
        <Empty>Pick two different services.</Empty>
      ) : q.isError ? (
        <ErrorState error={q.error} onRetry={() => q.refetch()} />
      ) : !r ? (
        <Loading label="Measuring…" />
      ) : r.path.length === 0 ? (
        <Empty>
          No messages flow from {nodeName(start)} to {nodeName(end)} in this range.
        </Empty>
      ) : (
        <>
          <div className="dsm-measure-path">
            <PathwayChain nodes={r.path} />
          </div>
          <div className="dsm-cards">
            <div className="dsm-card">
              <span>Latency p50</span>
              <b>{fmtMs(r.latencyP50Ms)}</b>
            </div>
            <div className="dsm-card">
              <span>Latency p99</span>
              <b>{fmtMs(r.latencyP99Ms)}</b>
            </div>
            <div className="dsm-card">
              <span>Throughput at {nodeName(end)}</span>
              <b>{fmtRate(r.throughput)}</b>
            </div>
            <div className="dsm-card">
              <span>Hops</span>
              <b>{(r.path.length - 1) / 2}</b>
            </div>
          </div>
          <LineChart
            title={`Latency from ${nodeName(start)} to ${nodeName(end)}`}
            fmt={fmtMs}
            height={260}
            series={[
              { name: 'p50', points: r.series.map((p) => [p.t, p.p50Ms]), color: '--chart-1' },
              { name: 'p99', points: r.series.map((p) => [p.t, p.p99Ms]), color: '--chart-4' },
            ]}
          />
          <p className="muted dsm-note">
            Each point adds up the hops' latencies in that interval; intervals where a hop has no messages are left out. Time spent inside services between hops is not counted.
          </p>
        </>
      )}
    </div>
  );
}
