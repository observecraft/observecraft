import { useState } from 'react';
import type { components } from '../../api/generated/schema';
import { fillColor, mapGroups } from './hostMapLayout';

type Host = components['schemas']['HostListItem'];
type Fill = 'cpu' | 'memory';

/** Hosts as hexagons, grouped like the list, filled by CPU or memory usage; a hexagon opens the host. */
export function HostMap({ hosts, onOpen, selected }: { hosts: Host[]; onOpen: (name: string) => void; selected: string | null }) {
  const [fill, setFill] = useState<Fill>('cpu');
  const value = (h: Host) => (fill === 'cpu' ? h.cpuPct : h.memPct);
  return (
    <div className="host-map">
      <div className="host-map-toolbar">
        <span className="muted">Fill by</span>
        <div className="btn-group">
          {(['cpu', 'memory'] as const).map((f) => (
            <button key={f} className={`btn${fill === f ? ' btn-active' : ''}`} onClick={() => setFill(f)}>
              {f === 'cpu' ? 'CPU usage %' : 'Memory usage %'}
            </button>
          ))}
        </div>
        <span className="host-map-legend">
          <span>0%</span>
          <span className="host-map-scale" />
          <span>100%</span>
        </span>
      </div>
      {mapGroups(hosts).map((g) => (
        <section key={g.key} className="host-map-group">
          <h4>
            {g.key} <span className="muted">{g.hosts.length}</span>
          </h4>
          <div className="host-map-hexes">
            {g.hosts.map((h) => {
              const v = value(h);
              return (
                <button
                  key={h.name}
                  className={`host-hex${selected === h.name ? ' selected' : ''}`}
                  style={{ background: fillColor(v) }}
                  title={`${h.name}\n${fill === 'cpu' ? 'CPU' : 'Memory'}: ${v == null ? 'no data' : `${v.toFixed(1)}%`}`}
                  aria-label={h.name}
                  onClick={() => onOpen(h.name)}
                />
              );
            })}
          </div>
        </section>
      ))}
    </div>
  );
}
