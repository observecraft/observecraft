import { useState } from 'react';
import { Link, useSearchParams } from 'react-router-dom';
import { ArrowLeft, PackagePlus, Search } from 'lucide-react';
import { CopyButton } from '../settings/SecuritySettings';
import { fill, sourcesFor, type GuideKind } from './sources';
import './setup.css';

const intakeUrl = () => `${window.location.protocol}//${window.location.hostname}:4318`;

const TITLES: Record<GuideKind, { title: string; intro: string }> = {
  logs: { title: 'Add a Log Source', intro: 'Choose where your logs come from. Every source sends to the ObserveCraft intake with an API key.' },
  infrastructure: { title: 'Add Resources', intro: 'Choose how to monitor your hosts, containers and clusters. Every agent sends to the ObserveCraft intake with an API key.' },
};

/** Logs → Add a Log Source and Infrastructure → Add Resources: pick a source, follow its steps. */
export function SetupGuidePage({ kind }: { kind: GuideKind }) {
  const [params, setParams] = useSearchParams();
  const [text, setText] = useState('');
  const sources = sourcesFor(kind);
  const selected = sources.find((s) => s.id === params.get('source'));
  const values = { intake: intakeUrl(), apiKey: '<API_KEY>' };
  const t = text.trim().toLowerCase();
  const shown = sources.filter((s) => !t || `${s.name} ${s.category} ${s.summary}`.toLowerCase().includes(t));
  const categories = [...new Set(shown.map((s) => s.category))];
  return (
    <div className="setup-page">
      <header className="product-header">
        <div className="product-title">
          <PackagePlus size={20} /> {TITLES[kind].title}
        </div>
      </header>
      <div className="setup-body">
        {selected ? (
          <section className="setup-guide">
            <button className="link-btn" onClick={() => setParams({})}>
              <ArrowLeft size={14} /> All sources
            </button>
            <h2>{selected.name}</h2>
            <p className="muted">{selected.summary}</p>
            <p className="setup-key-note">
              Replace <code>&lt;API_KEY&gt;</code> with a key from <Link to="/organization-settings/api-keys">API Keys</Link>. The intake for this install is <code>{values.intake}</code>.
            </p>
            <ol className="setup-steps">
              {selected.steps.map((step) => (
                <li key={step.title}>
                  <h3>{step.title}</h3>
                  {step.text && <p>{step.text}</p>}
                  {step.code && (
                    <div className="key-snippet">
                      <div className="key-snippet-head">
                        <span />
                        <CopyButton text={fill(step.code, values)} />
                      </div>
                      <pre>{fill(step.code, values)}</pre>
                    </div>
                  )}
                </li>
              ))}
            </ol>
          </section>
        ) : (
          <>
            <p className="muted">{TITLES[kind].intro}</p>
            <label className="tv-filter setup-search">
              <Search size={15} />
              <input placeholder="Search sources" value={text} onChange={(e) => setText(e.target.value)} />
            </label>
            {shown.length === 0 && <p className="muted">No sources match.</p>}
            {categories.map((c) => (
              <section key={c}>
                <h2 className="setup-category">{c}</h2>
                <div className="setup-tiles">
                  {shown
                    .filter((s) => s.category === c)
                    .map((s) => (
                      <button key={s.id} className="setup-tile" onClick={() => setParams({ source: s.id })}>
                        <strong>{s.name}</strong>
                        <span className="muted">{s.summary}</span>
                      </button>
                    ))}
                </div>
              </section>
            ))}
          </>
        )}
      </div>
    </div>
  );
}
