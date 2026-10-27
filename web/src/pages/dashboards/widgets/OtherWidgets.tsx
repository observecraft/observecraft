import { useQuery } from '@tanstack/react-query';
import { searchLogs } from '../../../api/client';
import type { WidgetDefinition } from '../../../api/dashboards/types';
import { formatLogDate } from '../../../lib/format';
import { STATUS_COLORS } from '../../../theme/chartTheme';
import { IntegrationLogo } from '../../integrations/IntegrationLogo';
import { LogMessage } from '../../logs/LogMessage';
import { useMonitorSummary } from '../queries';
import { useDashboardContext } from './context';
import { Markdown } from './markdown';
import { WidgetState } from './MetricWidgets';

type Def<T extends WidgetDefinition['type']> = Extract<WidgetDefinition, { type: T }>;

export function NoteWidget({ def }: { def: Def<'note'> }) {
  return (
    <div className={`note note-${def.background} note-${def.align} note-${def.fontSize}${def.pointer ? ' note-pointer' : ''}`}>
      <div className="note-body">
        <Markdown source={def.content} />
      </div>
    </div>
  );
}

export function ImageWidget({ def }: { def: Def<'image'> }) {
  if (def.src && /^(https?:|data:image\/|\/(?!\/))/.test(def.src)) {
    return <img className="img-widget" src={def.src} alt={def.title ?? ''} />;
  }
  return (
    <div className="img-widget img-logo" style={{ background: def.background ?? 'var(--bg-subtle)' }}>
      <span className="img-badge">{def.logo && <IntegrationLogo logo={def.logo} name={def.title ?? def.logo} size={110} />}</span>
    </div>
  );
}

const MON_LABEL = { alert: 'Alert', warn: 'Warn', ok: 'OK', nodata: 'No Data' } as const;

export function MonitorSummaryWidget({ def }: { def: Def<'monitor_summary'> }) {
  const { resolveSearch } = useDashboardContext();
  const q = useMonitorSummary(resolveSearch(def.query));
  const d = q.data;
  return (
    <WidgetState loading={q.isPending} error={q.error}>
      {d && (
        <div className="ms">
          <div className="ms-counts">
            {(Object.keys(MON_LABEL) as (keyof typeof MON_LABEL)[]).map((k) => (
              <div key={k} className={`ms-count ms-${k}`}>
                <span>{MON_LABEL[k]}</span>
                <strong>{d.counts[k].toLocaleString()}</strong>
              </div>
            ))}
          </div>
          <div className="ms-table">
            <div className="ms-head">
              <span>STATUS</span>
              <span>MONITOR NAME</span>
            </div>
            <div className="ms-rows">
              {d.monitors.slice(0, 200).map((m) => (
                <div key={m.id} className="ms-row">
                  <span className={`ms-pill ms-pill-${m.status}`}>{MON_LABEL[m.status].toUpperCase()}</span>
                  <span className="ms-name" title={m.name}>
                    {m.name}
                  </span>
                </div>
              ))}
            </div>
          </div>
        </div>
      )}
    </WidgetState>
  );
}

export function LogStreamWidget({ def }: { def: Def<'log_stream'> }) {
  const { range, resolveSearch } = useDashboardContext();
  const query = resolveSearch(def.query);
  const q = useQuery({ queryKey: ['logs', 'widget', query, range], queryFn: () => searchLogs({ range, query }) });
  const logs = q.data?.logs.slice(0, def.limit) ?? [];
  return (
    <WidgetState loading={q.isPending} error={q.error} empty={logs.length === 0}>
      <div className="ls">
        {logs.map((l) => (
          <div key={l.id} className="ls-row">
            <span className="status-bar" style={{ background: `var(${STATUS_COLORS[l.status]})` }} />
            <span className="mono ls-date">{formatLogDate(l.timestamp)}</span>
            <span className="mono ls-svc">{l.service}</span>
            <span className="mono ls-msg">
              <LogMessage message={l.message} />
            </span>
          </div>
        ))}
      </div>
    </WidgetState>
  );
}
