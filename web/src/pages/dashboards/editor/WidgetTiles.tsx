import type { WidgetType } from '../../../api/dashboards/types';

export const WIDGET_LABEL: Record<WidgetType, string> = {
  integration: 'Integration Panel',
  timeseries: 'Timeseries',
  query_value: 'Query Value',
  toplist: 'Top List',
  table: 'Table',
  pie: 'Pie Chart',
  bar_chart: 'Bar Chart',
  treemap: 'Treemap',
  distribution: 'Distribution',
  heatmap: 'Heatmap',
  change: 'Change',
  note: 'Note & Links',
  image: 'Image',
  group: 'Group',
  monitor_summary: 'Monitor Summary',
  log_stream: 'Log Stream',
};

export const WIDGET_SECTIONS: { title: string; types: WidgetType[] }[] = [
  { title: 'Graphs', types: ['timeseries', 'query_value', 'toplist', 'table', 'pie', 'bar_chart', 'treemap', 'distribution', 'heatmap', 'change'] },
  { title: 'Annotations & Embeds', types: ['note', 'image', 'group'] },
  { title: 'Monitors & Logs', types: ['monitor_summary', 'log_stream'] },
];

/** Small purple illustrations for the widget tray (original artwork). */
export function WidgetIllustration({ type }: { type: WidgetType }) {
  const g = 'url(#wt-grad)';
  const body = (() => {
    switch (type) {
      case 'timeseries':
        return (
          <>
            <polyline points="6,34 14,22 20,28 28,12 36,26 42,18 50,24 58,10" fill="none" stroke={g} strokeWidth="3" strokeLinejoin="round" />
            {[8, 16, 24, 32, 40, 48, 56].map((x, i) => (
              <rect key={x} x={x} y={44 - [6, 10, 8, 14, 9, 12, 7][i]} width="5" height={[6, 10, 8, 14, 9, 12, 7][i]} fill={g} opacity="0.75" />
            ))}
          </>
        );
      case 'query_value':
        return (
          <>
            <rect x="8" y="8" width="48" height="34" rx="3" fill={g} />
            <text x="32" y="32" textAnchor="middle" fontSize="15" fontWeight="700" fill="#fff">
              3.14
            </text>
          </>
        );
      case 'toplist':
        return [36, 28, 22, 14].map((w, i) => <rect key={i} x="12" y={10 + i * 9} width={w + 8} height="5" rx="1" fill={g} />);
      case 'table':
        return (
          <>
            <rect x="10" y="6" width="44" height="7" fill={g} />
            {[0, 1, 2, 3].map((i) => (
              <rect key={i} x="10" y={17 + i * 7} width="44" height="4" fill={g} opacity="0.6" />
            ))}
          </>
        );
      case 'pie':
        return (
          <>
            <circle cx="32" cy="25" r="17" fill="none" stroke={g} strokeWidth="9" strokeDasharray="60 47" />
            <circle cx="32" cy="25" r="17" fill="none" stroke="#e7dcff" strokeWidth="9" strokeDasharray="47 60" strokeDashoffset="-60" />
          </>
        );
      case 'bar_chart':
        return [14, 24, 18, 30, 22].map((h, i) => <rect key={i} x={12 + i * 9} y={42 - h} width="6" height={h} fill={g} />);
      case 'treemap':
        return (
          <>
            <rect x="8" y="8" width="26" height="34" fill={g} />
            <rect x="36" y="8" width="20" height="16" fill={g} opacity="0.7" />
            <rect x="36" y="26" width="20" height="16" fill={g} opacity="0.5" />
          </>
        );
      case 'distribution':
        return <polygon points="8,42 18,40 24,30 30,14 36,24 42,32 50,40 56,42" fill={g} />;
      case 'heatmap':
        return [0, 1, 2, 3].flatMap((r) =>
          [0, 1, 2, 3, 4, 5].map((c) => <rect key={`${r}${c}`} x={8 + c * 8} y={8 + r * 9} width="7" height="8" fill={g} opacity={0.25 + ((r * 3 + c * 5) % 7) / 10} />),
        );
      case 'change':
        return (
          <>
            {[30, 22, 14].map((w, i) => (
              <rect key={i} x="12" y={12 + i * 10} width={w} height="6" rx="3" fill={g} />
            ))}
            <path d="M50 30 l5 -8 l5 8z" fill="#2ea86b" />
          </>
        );
      case 'note':
        return (
          <>
            <rect x="10" y="6" width="44" height="34" rx="2" fill="#fff4c2" stroke="#f2c94c" />
            {[0, 1, 2].map((i) => (
              <rect key={i} x="16" y={14 + i * 7} width={30 - i * 6} height="3" fill="#b8902a" />
            ))}
          </>
        );
      case 'image':
        return (
          <>
            <rect x="8" y="6" width="48" height="36" rx="3" fill={g} opacity="0.35" />
            <polygon points="12,38 26,20 36,32 42,26 52,38" fill={g} />
            <circle cx="44" cy="15" r="4" fill={g} />
          </>
        );
      case 'group':
        return (
          <>
            <rect x="6" y="6" width="52" height="8" fill={g} />
            <rect x="6" y="17" width="25" height="24" fill={g} opacity="0.35" />
            <rect x="33" y="17" width="25" height="24" fill={g} opacity="0.35" />
          </>
        );
      case 'monitor_summary':
        return (
          <>
            {['#e5383b', '#f2a93b', '#2ea86b'].map((c, i) => (
              <rect key={c} x="8" y={8 + i * 12} width="10" height="7" rx="1" fill={c} />
            ))}
            {[0, 1, 2].map((i) => (
              <rect key={i} x="22" y={9 + i * 12} width="32" height="5" fill={g} opacity="0.6" />
            ))}
          </>
        );
      case 'log_stream':
        return [0, 1, 2, 3, 4].map((i) => (
          <g key={i}>
            <rect x="8" y={7 + i * 7} width="3" height="5" fill={i === 1 ? '#e5383b' : '#9cc3ea'} />
            <rect x="14" y={8 + i * 7} width={36 - (i % 3) * 6} height="3" fill={g} opacity="0.6" />
          </g>
        ));
    }
  })();
  return (
    <svg viewBox="0 0 64 48" width="64" height="48" aria-hidden>
      <defs>
        <linearGradient id="wt-grad" x1="0" y1="0" x2="1" y2="1">
          <stop offset="0" stopColor="#0b6e69" />
          <stop offset="1" stopColor="#29a8a0" />
        </linearGradient>
      </defs>
      {body}
    </svg>
  );
}
