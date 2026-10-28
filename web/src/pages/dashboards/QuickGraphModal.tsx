import { useMemo, useState } from 'react';
import { useNavigate } from 'react-router-dom';
import { LayoutDashboard, X } from 'lucide-react';
import { getDashboard } from '../../api/dashboards/client';
import type { Widget } from '../../api/dashboards/types';
import { TimePicker } from '../../components/shell/PageHeader';
import { useTime } from '../../components/shell/TimeContext';
import { Modal } from '../../components/ui/Modal';
import { AGGS, MetricPicker, toQuery, type Agg, type Row } from '../metrics/MetricsExplorer';
import { DashboardPickerModal } from './DashboardPickerModal';
import { useCreateDashboard, useMetricCatalog, useUpdateDashboardWidgets } from './queries';
import { DEFAULT_SIZE, bottom, newWidgetId } from './tree';
import { DashboardContext, type DashboardContextValue } from './widgets/context';
import { TimeseriesWidget } from './widgets/MetricWidgets';
import '../metrics/metrics.css';
import './reports/reports.css';

/** Graph one metric query without a dashboard, then optionally save it to one as a timeseries widget. */
export function QuickGraphModal({ onClose }: { onClose: () => void }) {
  const navigate = useNavigate();
  const { range } = useTime();
  const catalog = useMetricCatalog();
  const [row, setRow] = useState<Row>({ agg: 'avg', metric: '', from: '', by: '' });
  const [title, setTitle] = useState('');
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const create = useCreateDashboard();
  const append = useUpdateDashboardWidgets();
  const query = row.metric ? toQuery(row) : '';
  const ctx = useMemo<DashboardContextValue>(
    () => ({ range, resolve: (q) => q, resolveSearch: (q) => q, canEdit: false, onEdit: () => {}, onClone: () => {}, onDelete: () => {}, onFullscreen: () => {}, onGroupLayout: () => {} }),
    [range],
  );
  const tags = catalog.data?.find((m) => m.name === row.metric)?.tags ?? [];
  const widget = (y: number): Widget => ({
    id: newWidgetId(),
    definition: { type: 'timeseries', title: title.trim() || row.metric, display: 'line', requests: [{ query }] },
    layout: { x: 0, y, ...DEFAULT_SIZE.timeseries },
  });
  const saveTo = async (id: string) => {
    setError(null);
    try {
      const d = await getDashboard(id);
      await append.mutateAsync({ id, widgets: [...d.widgets, widget(bottom(d.widgets))] });
      onClose();
      navigate(`/dashboards/${id}`);
    } catch (e) {
      setError((e as Error).message);
    }
  };
  return (
    <>
      <Modal onClose={onClose} label="Quick Graph" width={980} height="auto">
        <div className="small-modal qg-modal">
          <div className="qg-head">
            <h3>Quick Graph</h3>
            <TimePicker />
            <button className="icon-plain" title="Close" onClick={onClose}>
              <X size={18} />
            </button>
          </div>
          <div className="mx-row">
            <select className="mx-agg" value={row.agg} onChange={(e) => setRow({ ...row, agg: e.target.value as Agg })} title="Space aggregation: how series are combined">
              {AGGS.map((a) => (
                <option key={a} value={a}>
                  {a}
                </option>
              ))}
            </select>
            <MetricPicker value={row.metric} catalog={catalog.data ?? []} onChange={(metric) => setRow({ ...row, metric })} />
            <label className="mx-field">
              from
              <input placeholder="everywhere (e.g. env:prod)" value={row.from} onChange={(e) => setRow({ ...row, from: e.target.value })} list="qg-tags" />
            </label>
            <label className="mx-field">
              by
              <input placeholder="(everything) e.g. service" value={row.by} onChange={(e) => setRow({ ...row, by: e.target.value })} list="qg-tags" />
            </label>
            <datalist id="qg-tags">
              {tags.map((t) => (
                <option key={t} value={t} />
              ))}
            </datalist>
          </div>
          {query && <code className="mx-query">{query}</code>}
          <div className="qg-chart">
            {query ? (
              <DashboardContext.Provider value={ctx}>
                <TimeseriesWidget def={{ type: 'timeseries', display: 'line', requests: [{ query }] }} />
              </DashboardContext.Provider>
            ) : (
              <div className="wd-state">Pick a metric to graph it.</div>
            )}
          </div>
          <div className="small-modal-actions">
            <input className="qg-title" placeholder="Graph title (optional)" value={title} onChange={(e) => setTitle(e.target.value)} />
            <button className="btn" onClick={onClose}>
              Close
            </button>
            <button className="btn btn-primary" disabled={!query} onClick={() => setSaving(true)}>
              <LayoutDashboard size={14} /> Save to dashboard
            </button>
          </div>
        </div>
      </Modal>
      {saving && (
        <DashboardPickerModal
          title="Save to dashboard"
          editableOnly
          busy={create.isPending || append.isPending}
          error={error ?? create.error?.message}
          onClose={() => setSaving(false)}
          onNew={() => create.mutate({ title: title.trim() || `Quick graph: ${row.metric}`, teams: [], layoutType: 'grid' }, { onSuccess: (d) => saveTo(d.id) })}
          onPick={(d) => saveTo(d.id)}
        />
      )}
    </>
  );
}
