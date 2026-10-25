import { useMemo } from 'react';
import { useQuery } from '@tanstack/react-query';
import { ArrowRight, ExternalLink, ScrollText, Waypoints, Wrench, type LucideIcon } from 'lucide-react';
import { useNavigate } from 'react-router-dom';
import { searchLogs } from '../../../api/client';
import type { HostDetail } from '../../../api/infrastructure/types';
import { CodeBlock } from '../../../components/code/CodeBlock';
import { useTime } from '../../../components/shell/TimeContext';
import { Empty, ErrorState, Loading } from '../../../components/ui/AsyncState';
import { snapRange } from '../../../lib/time';
import { LogTable } from '../../logs/LogTable';
import { HpSection } from './HpSection';

export function EmptySection({ id, title, icon, message }: { id: string; title: string; icon: LucideIcon; message: string }) {
  return (
    <HpSection id={id} title={title} icon={icon}>
      <div className="hp-empty">{message}</div>
    </HpSection>
  );
}

export function LogsSection({ host }: { host: HostDetail }) {
  const navigate = useNavigate();
  const { range: live } = useTime();
  const { from, to } = snapRange(live);
  const range = useMemo(() => ({ from, to }), [from, to]);
  const query = `host:${host.name}`;
  const q = useQuery({ queryKey: ['logs', 'host', host.name, range], queryFn: () => searchLogs({ range, query }) });
  return (
    <HpSection
      id="logs"
      title="Logs"
      icon={ScrollText}
      extra={
        <button className="btn" onClick={() => navigate(`/logs?query=${encodeURIComponent(query)}`)}>
          View in Log Explorer <ExternalLink size={13} />
        </button>
      }
    >
      {q.isPending ? (
        <Loading label="Loading logs…" />
      ) : q.isError ? (
        <ErrorState error={q.error} onRetry={() => q.refetch()} />
      ) : q.data.total === 0 ? (
        <Empty>No logs from this host in the selected time range.</Empty>
      ) : (
        <>
          <p className="hw-showing">
            Latest <strong>{Math.min(10, q.data.total)}</strong> of <strong>{q.data.total.toLocaleString()}</strong> logs
          </p>
          <div className="hp-logs">
            <LogTable logs={q.data.logs.slice(0, 10)} selectedId={null} onSelect={() => navigate(`/logs?query=${encodeURIComponent(query)}`)} options={{ wrap: false, showHost: false, showService: true }} />
          </div>
        </>
      )}
    </HpSection>
  );
}

export function AgentSection({ host }: { host: HostDetail }) {
  const rows: [string, string][] = [
    ['Collector', host.collectors.includes('observecraft') ? 'ObserveCraft Collector (OTel Collector + Vector)' : 'OpenTelemetry Collector'],
    ['Version', host.collectorVersion],
    ['Status', 'Running'],
    ['Config source', host.collectors.includes('observecraft') ? 'Fleet-managed' : 'Local file'],
    ['Integrations', host.integrations.map((i) => `${i.name}${i.status === 'broken' ? ' (broken)' : ''}`).join(', ') || '—'],
  ];
  return (
    <HpSection id="agent" title="Collector" icon={Wrench}>
      <dl className="hp-kv">
        {rows.map(([k, v]) => (
          <div key={k}>
            <dt>{k}</dt>
            <dd className={k === 'Status' ? 'hp-running' : undefined}>{v}</dd>
          </div>
        ))}
      </dl>
    </HpSection>
  );
}

export function CollectorSection({ host }: { host: HostDetail }) {
  return (
    <HpSection id="otel" title="OTel Pipelines" icon={Waypoints}>
      <div className="pipelines">
        {host.pipelines.map((p) => (
          <div key={p.name} className="pipeline">
            <span className="pipeline-name">{p.name}</span>
            <span className="pipeline-stage">
              {p.receivers.map((r) => (
                <span key={r} className="pl-chip pl-receiver">
                  {r}
                </span>
              ))}
            </span>
            <ArrowRight size={14} className="muted" />
            <span className="pipeline-stage">
              {p.processors.map((r) => (
                <span key={r} className="pl-chip pl-processor">
                  {r}
                </span>
              ))}
            </span>
            <ArrowRight size={14} className="muted" />
            <span className="pipeline-stage">
              {p.exporters.map((r) => (
                <span key={r} className="pl-chip pl-exporter">
                  {r}
                </span>
              ))}
            </span>
          </div>
        ))}
      </div>
      <CodeBlock code={host.collectorConfig} filename="/etc/observecraft/otelcol.yaml" />
    </HpSection>
  );
}
