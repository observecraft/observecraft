import { useMemo, useState } from 'react';
import type { EChartsOption } from 'echarts';
import { ArrowRight, Code2, Copy, Hexagon, ListTree, Plus, Puzzle, ScrollText, Server } from 'lucide-react';
import { useNavigate } from 'react-router-dom';
import type { HostDetail, TriggeredMonitor } from '../../../api/infrastructure/types';
import { Chart } from '../../../components/charts/Chart';
import { CodeBlock } from '../../../components/code/CodeBlock';
import { Modal } from '../../../components/ui/Modal';
import { hhmm } from '../../../lib/format';
import { token } from '../../../theme/chartTheme';
import { HpSection } from './HpSection';

const TAGS_SHOWN = 6;

export function TagsCard({ host }: { host: HostDetail }) {
  const [all, setAll] = useState(false);
  const tags = Object.entries(host.tags).sort(([a], [b]) => a.localeCompare(b));
  const shown = all ? tags : tags.slice(0, TAGS_SHOWN);
  return (
    <div className="hp-card hp-tags">
      <div className="hp-tags-row">
        <span className="hp-tags-label">OBSERVECRAFT TAGS</span>
        {shown.map(([k, v]) => (
          <span key={k} className="detail-tag">
            {k}:<span>{v}</span>
          </span>
        ))}
        {tags.length > TAGS_SHOWN && (
          <button className="detail-tag-more" onClick={() => setAll((a) => !a)}>
            {all ? 'less' : `+${tags.length - TAGS_SHOWN}`}
          </button>
        )}
      </div>
      <button className="btn hp-manage-tags" disabled title="User tags are not editable yet">
        <Plus size={14} /> Manage User Tags
      </button>
    </div>
  );
}

function monitorOption(m: TriggeredMonitor): EChartsOption {
  return {
    grid: { left: 34, right: 8, top: 10, bottom: 22 },
    tooltip: { trigger: 'axis' },
    xAxis: { type: 'time', axisLabel: { formatter: (v: number) => hhmm(v) } },
    yAxis: { type: 'value', min: 0, max: 100, splitNumber: 2 },
    series: [
      {
        type: 'line',
        data: m.series,
        color: token('--chart-1'),
        markArea: { silent: true, itemStyle: { color: 'rgba(229, 56, 59, 0.08)' }, data: [[{ yAxis: m.threshold }, { yAxis: 100 }]] },
        markLine: {
          silent: true,
          symbol: 'none',
          lineStyle: { type: 'dashed', color: token('--status-error') },
          label: { formatter: `y > ${m.threshold}%`, position: 'insideStartTop', color: token('--status-error'), fontSize: 11 },
          data: [{ yAxis: m.threshold }],
        },
      },
    ],
  };
}

function MonitorCard({ m }: { m: TriggeredMonitor }) {
  const navigate = useNavigate();
  const option = useMemo(() => monitorOption(m), [m]);
  return (
    <div className={`tm-card tm-${m.status}`}>
      <div className="tm-title">
        <span className={`tm-badge tm-badge-${m.status}`}>{m.status.toUpperCase()}</span>
        {m.name}
      </div>
      <Chart option={option} height={150} />
      <div className="tm-query mono">
        <span>{m.query}</span>
        <button className="icon-plain" title="Copy query" onClick={() => navigator.clipboard?.writeText(m.query)}>
          <Copy size={14} />
        </button>
      </div>
      <button className="btn btn-primary tm-view" onClick={() => navigate('/monitors')}>
        View Monitor
      </button>
    </div>
  );
}

const PRODUCT_ICON = { apm: ListTree, infra: Server, logs: ScrollText, processes: Hexagon } as const;

export function SummarySection({ host, onJump }: { host: HostDetail; onJump: (id: string) => void }) {
  const [showConfig, setShowConfig] = useState(false);
  const broken = host.integrations.filter((i) => i.status === 'broken');
  return (
    <HpSection
      id="summary"
      title="Host Summary"
      icon={Hexagon}
      extra={
        <button className="btn" onClick={() => setShowConfig(true)}>
          <Code2 size={14} /> View Configuration
        </button>
      }
    >
      <div className="hs-grid">
        <div>
          <h4 className="hs-h">
            Triggered Monitors <span className="et-tab-count">{host.triggeredMonitors.length}</span>
            {host.triggeredMonitors.length > 0 && (
              <a href="/monitors" className="hs-viewall">
                View All <ArrowRight size={14} />
              </a>
            )}
          </h4>
          {host.triggeredMonitors.length === 0 ? <p className="muted-text">No monitors are triggered on this host.</p> : host.triggeredMonitors.map((m) => <MonitorCard key={m.id} m={m} />)}
        </div>
        <div>
          <h4 className="hs-h">
            Issues <span className="et-tab-count">{broken.length}</span>
          </h4>
          {broken.length === 0 ? (
            <p className="muted-text">No issues detected.</p>
          ) : (
            <>
              <button className="hs-issue" onClick={() => onJump('metrics')}>
                <Puzzle size={16} />
                <strong>
                  {broken.length} Integration{broken.length > 1 ? 's' : ''} Broken
                </strong>
                <ArrowRight size={18} />
              </button>
              {broken.map((i) => (
                <div key={i.name} className="hs-issue-item">
                  <span className="hs-dot" /> {i.name}
                </div>
              ))}
            </>
          )}
        </div>
      </div>
      <div className="hs-products">
        <h4>ENABLED PRODUCTS</h4>
        <div>
          {host.products.map((p) => {
            const Icon = PRODUCT_ICON[p];
            return (
              <span key={p} className="hs-product">
                <Icon size={14} /> {p}
              </span>
            );
          })}
        </div>
      </div>
      {showConfig && (
        <Modal onClose={() => setShowConfig(false)} label="Collector configuration" width={900} height="auto">
          <div className="hp-config-modal">
            <h3>Collector configuration — {host.name}</h3>
            <CodeBlock code={host.collectorConfig} filename="/etc/observecraft/otelcol.yaml" />
          </div>
        </Modal>
      )}
    </HpSection>
  );
}
