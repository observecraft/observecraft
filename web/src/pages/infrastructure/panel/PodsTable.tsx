// The pods table of the host panel's Kubernetes section and of a service page's Infrastructure section.
import { Link } from 'react-router-dom';
import { ArrowUpNarrowWide, EllipsisVertical } from 'lucide-react';
import type { Pod } from '../../../api/infrastructure/types';
import { UsageBar, formatAge } from '../../../components/ui/Pager';
import '../infrastructure.css';

export function PodsTable({ pods, stale }: { pods: Pod[]; stale?: boolean }) {
  return (
    <table className={`hw-table${stale ? ' is-stale' : ''}`}>
      <thead>
        <tr>
          <th>
            <ArrowUpNarrowWide size={13} /> POD
          </th>
          <th>STATUS</th>
          <th>CLUSTER</th>
          <th>NAMESPACE</th>
          <th className="num">AGE</th>
          <th className="num">READY</th>
          <th className="num">RESTARTS</th>
          <th className="num">CPU USAGE/LIMITS</th>
          <th className="num">MEM USAGE/LIMITS</th>
          <th />
        </tr>
      </thead>
      <tbody>
        {pods.map((p) => (
          <tr key={p.uid}>
            <td className="hw-ellipsis">
              <Link to={`/orchestration/explorer?resource=pods&panel=${encodeURIComponent(p.uid)}`}>{p.name}</Link>
            </td>
            <td>
              <span className={`kx-status kx-${p.health}`}>{p.status}</span>
            </td>
            <td>
              <span className="hw-link-chip">{p.cluster}</span>
            </td>
            <td className="hw-ellipsis">
              <span className="hw-link-chip">{p.namespace}</span>
            </td>
            <td className="num">{p.ageSec == null ? '—' : formatAge(p.ageSec)}</td>
            <td className="num">
              <span className="ready-chip">{p.ready}</span>
            </td>
            <td className="num">{p.restarts}</td>
            <td className="num">
              {p.cpuPctOfLimit == null ? (
                '—'
              ) : (
                <>
                  {p.cpuPctOfLimit}% <UsageBar pct={p.cpuPctOfLimit} />
                </>
              )}
            </td>
            <td className="num">
              {p.memPctOfLimit == null ? (
                '—'
              ) : (
                <>
                  {p.memPctOfLimit}% <UsageBar pct={p.memPctOfLimit} tone="ok" />
                </>
              )}
            </td>
            <td>
              <EllipsisVertical size={15} className="muted" />
            </td>
          </tr>
        ))}
      </tbody>
    </table>
  );
}
