import { useEffect, useMemo } from 'react';
import { Link } from 'react-router-dom';
import type { EChartsOption } from 'echarts';
import { BellPlus, X } from 'lucide-react';
import type { WatchdogAlert } from '../../api/watchdog/client';
import { Chart } from '../../components/charts/Chart';
import { formatDuration, formatValue, monitorLink, relatedAlerts, valueLabel } from '../../lib/watchdog';
import { token } from '../../theme/chartTheme';

/** `Oct 28, 1:25 PM`. */
const at = (t: number) => new Date(t).toLocaleString(undefined, { month: 'short', day: 'numeric', hour: 'numeric', minute: '2-digit' });

/** The series with its expected range (a band) and the anomalous window shaded. */
function seriesOption(a: WatchdogAlert): EChartsOption {
  const pts = a.points;
  const color = a.status === 'ongoing' ? token('--status-error') : a.status === 'resolved' ? token('--status-ok') : token('--text-muted');
  const fmt = (v: number) => formatValue(v, a.unit);
  return {
    grid: { left: 56, right: 16, top: 16, bottom: 28 },
    tooltip: {
      trigger: 'axis',
      valueFormatter: (v) => (typeof v === 'number' ? fmt(v) : '-'),
    },
    xAxis: { type: 'time' },
    yAxis: { type: 'value', min: 0, axisLabel: { formatter: (v: number) => fmt(v) } },
    series: [
      { name: 'Expected low', type: 'line', stack: 'band', symbol: 'none', lineStyle: { opacity: 0 }, data: pts.map((p) => [p.t, p.lower]), tooltip: { show: false } },
      {
        name: 'Expected range',
        type: 'line',
        stack: 'band',
        symbol: 'none',
        lineStyle: { opacity: 0 },
        areaStyle: { color: token('--primary'), opacity: 0.12 },
        data: pts.map((p) => [p.t, p.upper === null || p.upper === undefined || p.lower === null || p.lower === undefined ? null : p.upper - p.lower]),
        tooltip: { show: false },
      },
      {
        name: valueLabel('type', a.alertType),
        type: 'line',
        symbol: 'none',
        connectNulls: false,
        lineStyle: { width: 2, color },
        itemStyle: { color },
        data: pts.map((p) => [p.t, p.value]),
        markArea: { silent: true, itemStyle: { color, opacity: 0.1 }, data: [[{ xAxis: a.start }, { xAxis: Math.min(a.end, pts[pts.length - 1]?.t ?? a.end) }]] },
      },
    ],
  };
}

/** One alert: what rose and by how much, the series against its expected range, related alerts and a monitor to catch it next time. */
export function AlertPanel({ alert, alerts, now, onOpen, onClose }: { alert: WatchdogAlert; alerts: WatchdogAlert[]; now: number; onOpen: (id: string) => void; onClose: () => void }) {
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => e.key === 'Escape' && onClose();
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  }, [onClose]);
  const option = useMemo(() => seriesOption(alert), [alert]);
  const related = relatedAlerts(alert, alerts);
  const upper = alert.points.find((p) => p.t >= alert.start)?.upper;
  return (
    <aside className="wd-panel" role="dialog" aria-label="Alert details">
      <div className={`wd-panel-scroll wd-edge-${alert.status}`}>
        <div className="wd-panel-head">
          <div>
            <div className="wd-panel-kicker">
              <span className={`wd-status-text wd-status-text-${alert.status}`}>{valueLabel('status', alert.status)}</span>
              <span className="muted-text">
                {valueLabel('category', alert.category)} · {valueLabel('type', alert.alertType)}
              </span>
            </div>
            <h2 className="wd-panel-title">{alert.title}</h2>
          </div>
          <button className="icon-btn" onClick={onClose} aria-label="Close">
            <X size={18} />
          </button>
        </div>

        <div className="wd-facts">
          <div>
            <span className="wd-fact-label">Started</span>
            {at(alert.start)}
          </div>
          <div>
            <span className="wd-fact-label">{alert.status === 'ongoing' ? 'Ongoing for' : 'Lasted'}</span>
            {formatDuration((alert.status === 'ongoing' ? now : alert.end) - alert.start)}
          </div>
          <div>
            <span className="wd-fact-label">Peak</span>
            <b>{formatValue(alert.peak, alert.unit)}</b>
          </div>
          <div>
            <span className="wd-fact-label">Usual</span>
            {formatValue(alert.baseline, alert.unit)}
            {upper !== null && upper !== undefined && <span className="muted-text"> (expected up to {formatValue(upper, alert.unit)})</span>}
          </div>
        </div>

        <section className="wd-section">
          <h3>
            {valueLabel('type', alert.alertType)}
            {alert.unit === 'logs' ? ' per 5 minutes' : ''}
          </h3>
          <Chart option={option} height={220} />
          <p className="muted-text wd-legend">
            <span className="wd-legend-band" /> Expected range, from the median and spread of the previous two hours
          </p>
        </section>

        <section className="wd-section">
          <h3>Related entities</h3>
          <ul className="wd-entities">
            {alert.service && (
              <>
                <li>
                  <span className="wd-tag">service:{alert.service}</span>
                  <Link to={`/apm/services/${encodeURIComponent(alert.service)}`}>APM service page</Link>
                  <Link to={`/logs?query=${encodeURIComponent(`service:${alert.service} status:error`)}`}>Error logs</Link>
                </li>
                {alert.team && (
                  <li>
                    <span className="wd-tag">team:{alert.team}</span>
                    <span className="muted-text">Owner, from service metadata</span>
                  </li>
                )}
              </>
            )}
            {alert.host && (
              <li>
                <span className="wd-tag">host:{alert.host}</span>
                <Link to={`/infrastructure?host=${encodeURIComponent(alert.host)}`}>Host details</Link>
                <Link to={`/logs?query=${encodeURIComponent(`host:${alert.host}`)}`}>Logs</Link>
              </li>
            )}
          </ul>
          {related.length > 0 && (
            <>
              <h4 className="wd-subhead">Other alerts on the same {alert.host ? 'host' : 'service'}</h4>
              <ul className="wd-related">
                {related.map((r) => (
                  <li key={r.id}>
                    <button className="link-btn" onClick={() => onOpen(r.id)}>
                      <span className={`wd-status-bar wd-status-${r.status}`} /> {r.title}
                    </button>
                    <span className="muted-text">{at(r.start)}</span>
                  </li>
                ))}
              </ul>
            </>
          )}
        </section>

        <section className="wd-section">
          <h3>Catch it next time</h3>
          <p className="muted-text">
            {alert.monitor.monitorType === 'log' ? 'A log' : alert.monitor.monitorType === 'apm' ? 'An APM' : 'A metric'} monitor alerting above {formatValue(alert.monitor.critical, alert.unit)}:
          </p>
          <code className="wd-query-code">{alert.monitor.query}</code>
          <Link className="btn btn-primary wd-create" to={monitorLink(alert)}>
            <BellPlus size={15} /> Create monitor
          </Link>
        </section>
      </div>
    </aside>
  );
}
