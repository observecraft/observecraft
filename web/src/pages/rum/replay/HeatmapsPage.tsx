import { useEffect, useMemo, useRef, useState } from 'react';
import { useSearchParams } from 'react-router-dom';
import { MousePointerClick } from 'lucide-react';
import type { HeatmapPoint } from '../../../api/rum/client';
import { TimePicker } from '../../../components/shell/PageHeader';
import { useTime } from '../../../components/shell/TimeContext';
import { Select } from '../../../components/ui/Dropdown';
import { Empty, ErrorState, Loading } from '../../../components/ui/AsyncState';
import { compact } from '../../../lib/format';
import { useSelectedApplication } from '../common';
import { useReplayHeatmap } from '../queries';
import { ReplayStage } from './common';
import { flatten, Replayer } from './replayer';

const minute = 60_000;

/** Heatmaps: where users click on a view, over the latest recorded snapshot of it. */
export function HeatmapsPage() {
  const [params, setParams] = useSearchParams();
  const [selected] = useSelectedApplication();
  const { range } = useTime();
  const viewName = params.get('view') ?? '';
  const frustration = params.get('frustration') ?? '';
  const set = (k: string, v: string) =>
    setParams(
      (p) => {
        const n = new URLSearchParams(p);
        if (v) n.set(k, v);
        else n.delete(k);
        return n;
      },
      { replace: true },
    );
  const r = useMemo(() => ({ from: Math.floor(range.from / minute) * minute, to: Math.ceil(range.to / minute) * minute }), [range.from, range.to]);
  const heatmap = useReplayHeatmap({ range: r, applicationId: selected, viewName: viewName || null, frustration: frustration || null });
  const data = heatmap.data;
  const views = data?.views ?? [];

  return (
    <div className="replay-page">
      <div className="replay-toolbar">
        <Select
          label="View Name"
          value={viewName}
          onChange={(v) => set('view', v)}
          options={[
            { value: '', label: 'Select a view' },
            ...views.map((v) => ({ value: v.name, label: v.name, hint: compact(v.count) })),
            ...(viewName && !views.some((v) => v.name === viewName) ? [{ value: viewName, label: viewName }] : []),
          ]}
        />
        <Select
          label="Clicks"
          value={frustration}
          onChange={(v) => set('frustration', v)}
          options={[
            { value: '', label: 'All clicks' },
            { value: 'rage_click', label: 'Rage clicks' },
            { value: 'dead_click', label: 'Dead clicks' },
            { value: 'error_click', label: 'Error clicks' },
          ]}
        />
        <span className="replay-spacer" />
        <TimePicker compact />
      </div>
      {heatmap.isLoading ? (
        <Loading />
      ) : heatmap.isError ? (
        <ErrorState error={heatmap.error} onRetry={() => heatmap.refetch()} />
      ) : !viewName ? (
        <section className="replay-getting-started">
          <h3>Getting Started</h3>
          <p className="muted">Pick a view to see where users click on it. The most visited views in this time range:</p>
          {views.length ? (
            <ol>
              {views.map((v) => (
                <li key={v.name}>
                  <button className="link-btn mono" onClick={() => set('view', v.name)}>
                    {v.name}
                  </button>
                  <span className="muted">{compact(v.count)} sessions</span>
                </li>
              ))}
            </ol>
          ) : (
            <p className="muted">No views in this time range.</p>
          )}
        </section>
      ) : !data?.snapshot?.length ? (
        <Empty>
          <b>No recording of {viewName} in this time range.</b>
          <p className="muted">Heatmaps are drawn over a recorded snapshot of the view. Widen the time range or pick another view.</p>
        </Empty>
      ) : (
        <div className="replay-heatmap">
          <Snapshot records={data.snapshot} clicks={data.clicks} />
          <aside className="replay-side">
            <h4 className="replay-side-h">
              <MousePointerClick size={14} /> Top elements
            </h4>
            {data.topElements.length ? (
              <ol className="replay-top">
                {data.topElements.map((t) => (
                  <li key={t.name}>
                    <span>{t.name || '(unnamed)'}</span>
                    <em style={{ width: `${(t.count / data.topElements[0].count) * 100}%` }} />
                    <b>{compact(t.count)}</b>
                  </li>
                ))}
              </ol>
            ) : (
              <p className="muted">No clicks on this view.</p>
            )}
            <p className="muted replay-count">{compact(data.clicks.length)} clicks drawn</p>
          </aside>
        </div>
      )}
    </div>
  );
}

/** The view's snapshot with the clicks on it as a heat layer. */
function Snapshot({ records, clicks }: { records: unknown[]; clicks: HeatmapPoint[] }) {
  const frame = useRef<HTMLIFrameElement>(null);
  const canvas = useRef<HTMLCanvasElement>(null);
  const [viewport, setViewport] = useState({ w: 1280, h: 800 });
  useEffect(() => {
    if (!frame.current) return;
    const recs = flatten([{ records }]);
    const r = new Replayer(frame.current, recs, { onViewport: (w, h) => setViewport({ w, h }) });
    r.seek(r.end);
  }, [records]);
  useEffect(() => {
    const c = canvas.current?.getContext('2d', { willReadFrequently: true });
    if (!c) return;
    c.clearRect(0, 0, viewport.w, viewport.h);
    // Accumulate soft spots as intensity, then color them from cool to hot.
    for (const p of clicks) {
      const g = c.createRadialGradient(p.x, p.y, 0, p.x, p.y, 28);
      g.addColorStop(0, 'rgba(0,0,0,0.25)');
      g.addColorStop(1, 'rgba(0,0,0,0)');
      c.fillStyle = g;
      c.fillRect(p.x - 28, p.y - 28, 56, 56);
    }
    const img = c.getImageData(0, 0, viewport.w, viewport.h);
    const px = img.data;
    for (let i = 0; i < px.length; i += 4) {
      const a = px[i + 3] / 255;
      if (!a) continue;
      // blue → green → yellow → red
      const [rr, gg, bb] =
        a < 0.33 ? [0, Math.round(a * 3 * 255), 255] : a < 0.66 ? [Math.round((a - 0.33) * 3 * 255), 255, Math.round(255 - (a - 0.33) * 3 * 255)] : [255, Math.round(255 - (a - 0.66) * 3 * 255), 0];
      px[i] = rr;
      px[i + 1] = gg;
      px[i + 2] = bb;
      px[i + 3] = Math.min(220, 60 + a * 255);
    }
    c.putImageData(img, 0, 0);
  }, [clicks, viewport]);
  return (
    <div className="replay-heatmap-stage">
      <ReplayStage viewport={viewport} frameRef={frame} fit="width">
        <canvas ref={canvas} width={viewport.w} height={viewport.h} />
      </ReplayStage>
    </div>
  );
}
