import type { ReactNode } from 'react';
import { Link } from 'react-router-dom';
import { ArrowRight, type LucideIcon } from 'lucide-react';
import './productIntro.css';

export interface IntroBenefit {
  icon: LucideIcon;
  title: string;
  text: string;
}

export interface IntroStart {
  icon: ReactNode;
  title: string;
  text: string;
  /** A route, or a click handler (e.g. open a setup dialog). */
  to?: string;
  onClick?: () => void;
  cta?: string;
}

/**
 * A product's page before it has data: a hero with what the product does and how to start, an optional
 * preview drawn from fictional data, what it helps with (LEARN MORE) and the ways to send data (GET STARTED).
 */
export function ProductIntro({
  icon: Icon,
  product,
  title,
  text,
  actions,
  preview,
  benefits,
  start,
  children,
}: {
  icon: LucideIcon;
  product: string;
  title: string;
  text: ReactNode;
  actions?: ReactNode;
  preview?: ReactNode;
  benefits?: IntroBenefit[];
  start?: IntroStart[];
  children?: ReactNode;
}) {
  return (
    <div className="pi-intro">
      <section className={`pi-hero${preview ? '' : ' centered'}`}>
        <div className="pi-hero-text">
          <div className="pi-kicker">
            <span className="pi-icon">
              <Icon size={18} />
            </span>
            {product}
          </div>
          <h1>{title}</h1>
          <p>{text}</p>
          {actions && <div className="pi-actions">{actions}</div>}
        </div>
        {preview && (
          <div className="pi-preview" aria-hidden>
            <div className="pi-preview-card">{preview}</div>
          </div>
        )}
      </section>
      {benefits && benefits.length > 0 && (
        <>
          <h3 className="pi-section-title">LEARN MORE</h3>
          <div className="pi-benefits" style={{ gridTemplateColumns: `repeat(${Math.min(benefits.length, 4)}, minmax(0, 1fr))` }}>
            {benefits.map((b) => (
              <div key={b.title} className="pi-benefit">
                <b.icon size={22} />
                <b>{b.title}</b>
                <p>{b.text}</p>
              </div>
            ))}
          </div>
        </>
      )}
      {start && start.length > 0 && (
        <>
          <h3 className="pi-section-title">GET STARTED</h3>
          <div className="pi-start" style={{ gridTemplateColumns: `repeat(${Math.min(start.length, 3)}, minmax(0, 1fr))` }}>
            {start.map((s) => {
              const body = (
                <>
                  {s.icon}
                  <b>{s.title}</b>
                  <p>{s.text}</p>
                  <span className="pi-start-link">
                    {s.cta ?? 'Set up'} <ArrowRight size={14} />
                  </span>
                </>
              );
              return s.to ? (
                <Link key={s.title} className="pi-start-card" to={s.to}>
                  {body}
                </Link>
              ) : (
                <button key={s.title} type="button" className="pi-start-card" onClick={s.onClick}>
                  {body}
                </button>
              );
            })}
          </div>
        </>
      )}
      {children}
    </div>
  );
}

/** Hero buttons: white on the hero's gradient, and outlined. */
export const introButton = { primary: 'btn pi-btn-white', secondary: 'btn pi-btn-outline' } as const;

/** A small dashboard drawn from fictional numbers, for a hero's preview. */
export function IntroPreview({ title, kpis, bars, rows }: { title: string; kpis?: [string, string][]; bars?: number[]; rows?: [string, string][] }) {
  const max = Math.max(1, ...(bars ?? []));
  return (
    <div className="pi-mini">
      <div className="pi-mini-title">{title}</div>
      {kpis && (
        <div className="pi-mini-kpis">
          {kpis.map(([label, value]) => (
            <div key={label}>
              <span>{label}</span>
              <b>{value}</b>
            </div>
          ))}
        </div>
      )}
      {bars && (
        <div className="pi-mini-bars">
          {bars.map((v, i) => (
            <span key={i} style={{ height: `${(v / max) * 100}%` }} />
          ))}
        </div>
      )}
      {rows && (
        <table className="pi-mini-rows">
          <tbody>
            {rows.map(([name, value]) => (
              <tr key={name}>
                <td>{name}</td>
                <td>{value}</td>
              </tr>
            ))}
          </tbody>
        </table>
      )}
    </div>
  );
}
