import { useEffect, useMemo, useState } from 'react';
import { createPortal } from 'react-dom';
import { Link, useNavigate } from 'react-router-dom';
import type { EChartsOption } from 'echarts';
import { BellPlus, Pencil, Plus, Trash2, X } from 'lucide-react';
import type { SloDetail, SloHistoryPoint, SloItem, SloStatus, SloWindow } from '../../api/slos/client';
import type { MonitorStatus } from '../../api/monitors/client';
import { Chart } from '../../components/charts/Chart';
import { ErrorState, Loading } from '../../components/ui/AsyncState';
import { token } from '../../theme/chartTheme';
import { StatusBadge, TagChips } from '../monitors/common';
import { CATEGORY_LABEL, STATE_COLOR, StateBadge, TYPE_LABEL, TypeIcon, WINDOW_LABEL, budgetLeft, burnRateMonitorLink, fmtBurn, fmtDateTime, fmtPct } from './common';
import { CorrectionModal } from './CorrectionModal';
import { useDeleteSlo, useSlo } from './queries';

/** The SLO side panel: status per target window, SLI and error budget charts, definition, monitors and corrections. */
export function SloPanel({ id, window, onClose }: { id: string; window?: SloWindow; onClose: () => void }) {
  const [picked, setPicked] = useState<SloWindow | undefined>(window);
  const q = useSlo(id, picked);
  useEffect(() => setPicked(window), [id, window]);
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => e.key === 'Escape' && onClose();
    globalThis.addEventListener('keydown', onKey);
    return () => globalThis.removeEventListener('keydown', onKey);
  }, [onClose]);

  return createPortal(
    <div className="slo-drawer-backdrop" onMouseDown={(e) => e.target === e.currentTarget && onClose()}>
      <aside className="slo-drawer" role="dialog" aria-label="SLO details">
        {q.isPending ? <Loading label="Loading SLO…" /> : q.isError ? <ErrorState error={q.error} onRetry={() => q.refetch()} /> : <PanelBody detail={q.data} onWindow={setPicked} onClose={onClose} />}
      </aside>
    </div>,
    document.body,
  );
}

function PanelBody({ detail, onWindow, onClose }: { detail: SloDetail; onWindow: (w: SloWindow) => void; onClose: () => void }) {
  const navigate = useNavigate();
  const remove = useDeleteSlo();
  const [correcting, setCorrecting] = useState(false);
  const { slo } = detail;
  const st = slo.status;
  const burnLink = burnRateMonitorLink(slo);
  const shown = detail.statuses.some((s) => s.window === st.window) ? detail.statuses : [...detail.statuses, st];

  return (
    <>
      <header className="slo-drawer-head">
        <TypeIcon type={slo.type} size={18} />
        <h2>{slo.name}</h2>
        <StateBadge state={st.state} />
        <span className="slo-drawer-actions">
          <button className="btn" onClick={() => navigate(`/slo/${slo.id}/edit`)}>
            <Pencil size={14} /> Edit
          </button>
          <button className="btn" onClick={() => setCorrecting(true)}>
            <Plus size={14} /> Correction
          </button>
          {burnLink ? (
            <Link className="btn" to={burnLink} title="Metric monitor on the error rate against 14.4× the allowed rate over the last hour">
              <BellPlus size={14} /> Burn rate alert
            </Link>
          ) : (
            <button className="btn" disabled title="Burn rate alerts are available for metric-based SLOs">
              <BellPlus size={14} /> Burn rate alert
            </button>
          )}
          <button
            className="btn"
            title="Delete"
            onClick={() => window.confirm(`Delete the SLO "${slo.name}"? This can't be undone.`) && remove.mutate(slo.id, { onSuccess: onClose })}
            aria-label="Delete SLO"
          >
            <Trash2 size={14} />
          </button>
          <button className="icon-plain" onClick={onClose} title="Close">
            <X size={18} />
          </button>
        </span>
      </header>
      <div className="slo-drawer-body">
        {slo.description && <p className="slo-desc">{slo.description}</p>}
        <div className="slo-meta">
          <span>{TYPE_LABEL[slo.type]}-based</span>
          {slo.creator && <span>Created by {slo.creator.name}</span>}
          <TagChips tags={[...slo.teams.map((t) => `team:${t}`), ...slo.tags]} max={8} />
        </div>
        {remove.isError && <p className="form-error">{(remove.error as Error).message}</p>}

        <div className="slo-windows">
          {shown.map((s) => (
            <WindowCard key={s.window} status={s} primary={s.window === slo.primaryWindow} active={s.window === st.window} onClick={() => onWindow(s.window)} />
          ))}
        </div>

        <section className="slo-section">
          <h3>
            Status over the past {WINDOW_LABEL[st.window]} <span className="muted">· target {fmtPct(st.target)}</span>
          </h3>
          <HistoryChart history={detail.history} status={st} kind="sli" />
        </section>
        <section className="slo-section">
          <h3>
            Error budget <span className="muted">· {budgetLeft(st) || 'no data'}</span>
          </h3>
          <HistoryChart history={detail.history} status={st} kind="budget" />
        </section>

        <section className="slo-section">
          <h3>Definition</h3>
          <Definition slo={slo} />
        </section>

        <section className="slo-section">
          <h3>Monitors</h3>
          {detail.monitors.length === 0 ? (
            <p className="muted">No monitors yet. {burnLink ? 'Create a burn rate alert to be notified before the budget runs out.' : ''}</p>
          ) : (
            <ul className="slo-list">
              {detail.monitors.map((m) => (
                <li key={m.id}>
                  <StatusBadge status={m.status as MonitorStatus} small />
                  <Link to={`/monitors/${m.id}`}>{m.name}</Link>
                </li>
              ))}
            </ul>
          )}
        </section>

        <section className="slo-section">
          <h3>Corrections in this window</h3>
          {detail.corrections.length === 0 ? (
            <p className="muted">None.</p>
          ) : (
            <ul className="slo-list">
              {detail.corrections.map((c) => (
                <li key={c.id}>
                  <span className="slo-corr-cat">{CATEGORY_LABEL[c.category]}</span>
                  <span>
                    {fmtDateTime(c.start)} – {fmtDateTime(c.end)}
                  </span>
                  <span className="muted">{c.sloId ? c.description : `${c.description} (every SLO)`}</span>
                </li>
              ))}
            </ul>
          )}
        </section>
      </div>
      {correcting && <CorrectionModal onClose={() => setCorrecting(false)} sloId={slo.id} slos={[slo]} />}
    </>
  );
}

function WindowCard({ status, primary, active, onClick }: { status: SloStatus; primary: boolean; active: boolean; onClick: () => void }) {
  return (
    <button className={`slo-window${active ? ' active' : ''}`} onClick={onClick} style={{ borderLeftColor: STATE_COLOR[status.state] }}>
      <span className="slo-window-head">
        {status.window}
        {primary && <span className="slo-primary">PRIMARY</span>}
      </span>
      <span className={`slo-window-sli slo-sli-${status.state}`}>{status.state === 'no_data' ? 'No Data' : fmtPct(status.sli)}</span>
      <span className="muted">
        Target {fmtPct(status.target)}
        {status.warning ? ` · warn ${fmtPct(status.warning)}` : ''}
      </span>
      <span>Budget {fmtPct(status.budgetRemaining, 1)}</span>
      <span className="muted">
        Burn {fmtBurn(status.burnRate1h)} (1h) · {fmtBurn(status.burnRate)} ({status.window})
      </span>
    </button>
  );
}

function Definition({ slo }: { slo: SloItem }) {
  if (slo.type === 'metric') {
    return (
      <dl className="slo-def">
        <dt>Good events</dt>
        <dd className="mono">{slo.goodQuery}</dd>
        <dt>Total events</dt>
        <dd className="mono">{slo.totalQuery}</dd>
      </dl>
    );
  }
  if (slo.type === 'time_slice') {
    return (
      <dl className="slo-def">
        <dt>Good slice</dt>
        <dd className="mono">
          {slo.sliceQuery} {slo.comparator} {slo.threshold}
        </dd>
        <dt>Slice</dt>
        <dd>
          {slo.sliceMinutes} minute{slo.sliceMinutes === 1 ? '' : 's'}
        </dd>
      </dl>
    );
  }
  return (
    <dl className="slo-def">
      <dt>Uptime</dt>
      <dd>Time none of the {slo.monitorIds.length} monitors below is in alert.</dd>
    </dl>
  );
}

function HistoryChart({ history, status, kind }: { history: SloHistoryPoint[]; status: SloStatus; kind: 'sli' | 'budget' }) {
  const option = useMemo((): EChartsOption => {
    const color = STATE_COLOR[status.state].startsWith('var(') ? token(STATE_COLOR[status.state].slice(4, -1)) || token('--primary') : STATE_COLOR[status.state];
    const values = history.map((h) => [h.t, kind === 'sli' ? (h.sli ?? null) : (h.budgetRemaining ?? null)]);
    const line = kind === 'sli' ? status.target : 0;
    const finite = values.map((v) => v[1]).filter((v): v is number => v !== null);
    const min = kind === 'sli' ? Math.min(line, ...finite) : Math.min(0, ...finite);
    return {
      grid: { left: 52, right: 16, top: 12, bottom: 24 },
      tooltip: { trigger: 'axis', valueFormatter: (v) => (typeof v === 'number' ? fmtPct(v, kind === 'sli' ? 3 : 1) : '—') },
      xAxis: { type: 'time' },
      yAxis: {
        type: 'value',
        min: kind === 'sli' ? Math.max(0, Math.floor((min - (100 - min) * 0.5) * 100) / 100) : Math.floor(min / 10) * 10,
        max: 100,
        axisLabel: { formatter: (v: number) => `${Number(v.toFixed(2))}%` },
      },
      series: [
        {
          type: 'line',
          name: kind === 'sli' ? 'SLI' : 'Budget left',
          data: values,
          showSymbol: false,
          lineStyle: { color, width: 2 },
          itemStyle: { color },
          areaStyle: kind === 'budget' ? { color, opacity: 0.12 } : undefined,
          markLine: {
            silent: true,
            symbol: 'none',
            lineStyle: { type: 'dashed', color: token('--text-faint') },
            data: [{ yAxis: line }],
            label: { position: 'insideEndTop', formatter: kind === 'sli' ? 'target' : 'empty' },
          },
        },
      ],
    };
  }, [history, status, kind]);
  if (history.length === 0) return <p className="muted slo-chart-empty">No data in this window.</p>;
  return <Chart option={option} height={170} />;
}
