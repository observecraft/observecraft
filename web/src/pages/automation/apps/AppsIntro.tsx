import { Link } from 'react-router-dom';
import { ArrowRight, Blocks, LayoutGrid, Play, ShieldCheck } from 'lucide-react';
import './apps.css';

const FEATURES = [
  { icon: Blocks, title: 'Drag in components', body: 'Headings, inputs, buttons, tables and metric values on a 12-column grid. What you see in the builder is what people get.' },
  { icon: Play, title: 'Buttons run workflows', body: 'A button starts a published workflow with the values people typed, then shows what the run returned.' },
  { icon: ShieldCheck, title: 'Safe to share', body: 'Runs act as the workflow’s publisher, credentials never leave the server, and every run is recorded.' },
];

/** App Builder intro: what apps are, with a drawing of one. */
export function AppsIntro() {
  return (
    <div className="ab-intro">
      <section className="ab-hero">
        <div className="ab-hero-kicker">
          <span className="ab-hero-badge">
            <LayoutGrid size={14} />
          </span>
          App Builder
        </div>
        <h1>
          Build custom apps
          <br />
          with your ObserveCraft data
        </h1>
        <p>Turn a workflow into a small tool your team can use: a form to fill, a button to press, and the results right below — next to live metrics.</p>
        <div className="ab-hero-cta">
          <Link className="ab-cta" to="/automation/apps/new">
            Create App <ArrowRight size={15} />
          </Link>
          <Link className="ab-cta-ghost" to="/automation/apps/blueprints">
            Browse blueprints
          </Link>
        </div>
        <AppDrawing />
      </section>
      <section className="ab-features">
        {FEATURES.map((f) => (
          <div key={f.title} className="ab-feature">
            <span className="ab-feature-icon">
              <f.icon size={18} />
            </span>
            <h3>{f.title}</h3>
            <p>{f.body}</p>
          </div>
        ))}
      </section>
    </div>
  );
}

/** Our own drawing of an app: a form, a run button, a results table and metric tiles. */
function AppDrawing() {
  return (
    <div className="ab-drawing" aria-hidden="true">
      <div className="ab-drawing-bar">
        <span className="ab-dot" />
        <span className="ab-dot" />
        <span className="ab-dot" />
        <span className="ab-drawing-title">Service operations</span>
        <span className="ab-drawing-chip">Published</span>
      </div>
      <div className="ab-drawing-body">
        <div className="ab-drawing-tiles">
          {[
            ['storefront-api req/s', '1,284'],
            ['order-service errors/s', '0.42'],
            ['p95 latency', '212 ms'],
          ].map(([l, v]) => (
            <div key={l} className="ab-drawing-tile">
              <span>{l}</span>
              <strong>{v}</strong>
            </div>
          ))}
        </div>
        <div className="ab-drawing-form">
          {['Service', 'Environment', 'Replicas'].map((l, i) => (
            <div key={l} className="ab-drawing-field">
              <span>{l}</span>
              <i>{['order-service', 'staging', '4'][i]}</i>
            </div>
          ))}
          <div className="ab-drawing-btn">▶ Scale service</div>
        </div>
        <div className="ab-drawing-table">
          <div className="ab-drawing-row head">
            <span>namespace</span>
            <span>pods</span>
            <span>status</span>
          </div>
          {[
            ['storefront', '4', 'running'],
            ['storefront-canary', '1', 'running'],
          ].map((r) => (
            <div key={r[0]} className="ab-drawing-row">
              <span>{r[0]}</span>
              <span>{r[1]}</span>
              <span className="ok">{r[2]}</span>
            </div>
          ))}
        </div>
      </div>
    </div>
  );
}
