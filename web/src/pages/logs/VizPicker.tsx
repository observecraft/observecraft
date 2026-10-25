import {
  BarChart2,
  LineChart,
  List,
  ListOrdered,
  PieChart,
  ScatterChart,
  SquareStack,
  Table2,
  Triangle,
} from 'lucide-react';
import type { VizMode } from './AggregateView';

const ALL: { key: VizMode; label: string; icon: typeof List }[] = [
  { key: 'list', label: 'List', icon: List },
  { key: 'timeseries', label: 'Timeseries', icon: LineChart },
  { key: 'toplist', label: 'Top List', icon: ListOrdered },
  { key: 'bar', label: 'Bar Chart', icon: BarChart2 },
  { key: 'table', label: 'Table', icon: Table2 },
  { key: 'treemap', label: 'Tree Map', icon: SquareStack },
  { key: 'pie', label: 'Pie Chart', icon: PieChart },
  { key: 'scatter', label: 'Scatter Plot', icon: ScatterChart },
  { key: 'distribution', label: 'Distribution', icon: Triangle },
];

/** "Visualize as" segmented control. Live Tail offers a subset of the explorer's modes. */
export function VizPicker({ value, onChange, modes }: { value: VizMode; onChange: (m: VizMode) => void; modes?: VizMode[] }) {
  const items = modes ? ALL.filter((m) => modes.includes(m.key)) : ALL;
  return (
    <div className="viz-row">
      <div className="btn-group">
        <span className="btn btn-label">Visualize as</span>
        {items.map(({ key, label, icon: Icon }) => (
          <button key={key} className={`btn${value === key ? ' btn-active' : ''}`} onClick={() => onChange(key)}>
            <Icon size={14} /> {label}
          </button>
        ))}
      </div>
    </div>
  );
}
