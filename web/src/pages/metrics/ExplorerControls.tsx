import { useState } from 'react';
import { useNavigate } from 'react-router-dom';
import { Check, ChevronDown, FileSearch, Gauge, Network, Palette } from 'lucide-react';
import { usePopover } from '../../components/ui/Dropdown';
import { PALETTES, seriesColor, seriesPalette } from '../../theme/chartTheme';

/** Where a query reads from: metrics here; logs and spans in their own explorers' analytics. */
const SOURCES = [
  { key: 'metrics', label: 'Metrics', icon: Gauge, to: null },
  { key: 'spans', label: 'APM Spans', icon: Network, to: '/apm/traces?viz=timeseries' },
  { key: 'logs', label: 'Logs', icon: FileSearch, to: '/logs?viz=timeseries' },
] as const;

export function SourcePicker() {
  const { open, setOpen, ref } = usePopover();
  const navigate = useNavigate();
  return (
    <div className="dd" ref={ref}>
      <button type="button" className="mx-source" onClick={() => setOpen((o) => !o)}>
        Metrics <ChevronDown size={13} />
      </button>
      {open && (
        <ul className="dd-menu dd-left mx-source-menu">
          <li className="mx-menu-head">POPULAR</li>
          {SOURCES.map((s) => (
            <li key={s.key}>
              <button
                type="button"
                className={s.key === 'metrics' ? 'selected' : undefined}
                onClick={() => {
                  setOpen(false);
                  if (s.to) navigate(s.to);
                }}
              >
                <s.icon size={15} /> {s.label}
              </button>
            </li>
          ))}
        </ul>
      )}
    </div>
  );
}

/** A palette's first colors, as the pickers show them. */
export function Swatch({ name, count = 3 }: { name: string; count?: number }) {
  return (
    <span className="mx-swatch">
      {Array.from({ length: count }, (_, i) => (
        <i key={i} style={{ background: name === 'consistent' ? seriesColor('classic', '', i * 2) : seriesPalette(name)[i] }} />
      ))}
    </span>
  );
}

/** The graph's palette: a list with each palette's colors and, beside it, what the hovered one is for. */
export function PalettePicker({ value, onChange }: { value: string; onChange: (p: string) => void }) {
  const { open, setOpen, ref } = usePopover();
  const [hover, setHover] = useState(value);
  const current = PALETTES.find((p) => p.name === value) ?? PALETTES[1];
  const described = PALETTES.find((p) => p.name === hover) ?? current;
  return (
    <div className="dd" ref={ref}>
      <button type="button" className="mx-inline-select" onClick={() => setOpen((o) => !o)}>
        <Swatch name={current.name} /> {current.label} <ChevronDown size={13} />
      </button>
      {open && (
        <div className="dd-menu dd-left mx-palette-menu">
          <ul>
            {PALETTES.map((p) => (
              <li key={p.name} onMouseEnter={() => setHover(p.name)}>
                <button
                  type="button"
                  className={p.name === value ? 'selected' : undefined}
                  onClick={() => {
                    onChange(p.name);
                    setOpen(false);
                  }}
                >
                  <Swatch name={p.name} /> {p.label}
                  {p.name === value && <Check size={14} className="mx-check" />}
                </button>
              </li>
            ))}
          </ul>
          <p>{described.description}</p>
        </div>
      )}
    </div>
  );
}

/** One query's color override: a palette for its series, or the graph's. */
export function ColorOverride({ value, onChange }: { value?: string; onChange: (p: string | undefined) => void }) {
  const { open, setOpen, ref } = usePopover();
  return (
    <div className="dd" ref={ref}>
      <button type="button" className={`icon-plain mx-override-btn${value ? ' active' : ''}`} title="Color override" onClick={() => setOpen((o) => !o)}>
        <Palette size={16} />
      </button>
      {open && (
        <div className="dd-menu dd-right mx-override">
          <div className="mx-override-head">
            <b>Color Override</b>
            <button
              type="button"
              className="link-btn"
              onClick={() => {
                onChange(undefined);
                setOpen(false);
              }}
            >
              Reset
            </button>
          </div>
          {PALETTES.filter((p) => p.name !== 'consistent').map((p) => (
            <button
              key={p.name}
              type="button"
              className={`mx-override-row${value === p.name ? ' selected' : ''}`}
              onClick={() => {
                onChange(p.name);
                setOpen(false);
              }}
            >
              <span>{p.label}</span>
              <Swatch name={p.name} count={6} />
            </button>
          ))}
        </div>
      )}
    </div>
  );
}
