import { Link } from 'react-router-dom';
import { ArrowRight, Clock, FileCode2, Plug, Sparkles, Tags, Workflow } from 'lucide-react';

/** The catalog before anything is known: instrument with APM, or one of three other ways in. */
export function CatalogWelcome({ onImport }: { onImport: () => void }) {
  return (
    <div className="pc-welcome">
      <h1 className="pc-welcome-title">Welcome to Software Catalog</h1>
      <p className="pc-welcome-sub">Select a method to start populating catalog entries</p>

      <section className="pc-featured">
        <div className="pc-featured-icon">
          <Workflow size={22} />
        </div>
        <div className="pc-featured-body">
          <div className="pc-featured-head">
            <h2>Instrument a service with APM</h2>
            <span className="pc-pill pc-pill-accent">
              <Sparkles size={12} /> Recommended
            </span>
          </div>
          <p>
            Traced services appear here on their own, together with the databases, caches, queues and third-party APIs they call, their endpoints, versions and performance. Owners, links and docs can
            be added afterwards.
          </p>
          <div className="pc-featured-foot">
            <Link className="btn btn-primary" to="/apm?instrument=1">
              Get Started <ArrowRight size={15} />
            </Link>
            <span className="pc-footnote">
              <Clock size={13} /> A few minutes per service
            </span>
            <span className="pc-footnote">OpenTelemetry or Datadog tracing libraries, unchanged</span>
          </div>
        </div>
      </section>

      <div className="pc-divider">
        <span>Alternative methods</span>
      </div>

      <div className="pc-methods">
        <Link className="pc-method" to="/portal/settings">
          <Plug size={20} />
          <h3>Connect integrations</h3>
          <p>Bring owners and teams from the tools that already know them, like your code host or incident tooling.</p>
          <span className="pc-method-cta">
            Open settings <ArrowRight size={14} />
          </span>
        </Link>
        <Link className="pc-method" to="/logs">
          <Tags size={20} />
          <h3>Extract entities</h3>
          <p>
            Services named by the <code>service</code> tag on logs and metrics are listed as inferred entities, ready to be claimed.
          </p>
          <span className="pc-method-cta">
            Check your tags <ArrowRight size={14} />
          </span>
        </Link>
        <button className="pc-method" onClick={onImport}>
          <FileCode2 size={20} />
          <h3>Import metadata</h3>
          <p>
            Paste or upload entity definition files (<code>apiVersion: v3</code>, YAML or JSON) kept next to your code.
          </p>
          <span className="pc-method-cta">
            Import definitions <ArrowRight size={14} />
          </span>
        </button>
      </div>
    </div>
  );
}
