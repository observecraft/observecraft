import { Link, useSearchParams } from 'react-router-dom';
import { ArrowLeft, Boxes, Cloud, PackagePlus, Plug, Server, type LucideIcon } from 'lucide-react';
import { CopyButton } from '../settings/SecuritySettings';
import { categoriesFor, fill, sourcesFor, type CategoryName, type GuideKind } from './sources';
import './setup.css';

const intakeUrl = () => `${window.location.protocol}//${window.location.hostname}:4318`;

const ICONS: Record<CategoryName, LucideIcon> = { Server, Container: Boxes, Cloud, Other: Plug };

const TITLES: Record<GuideKind, { title: string; question: string; start: string; intro: string }> = {
  logs: {
    title: 'Add a Log Source',
    question: 'Where are your logs coming from?',
    start: 'Get started with logs',
    intro: 'Pick where your logs come from and follow its steps. Once they arrive, search them in the Log Explorer and pivot to the metrics and traces of the same service.',
  },
  infrastructure: {
    title: 'Add Resources',
    question: 'What do you want to monitor?',
    start: 'Get started with infrastructure',
    intro: 'Pick where your workloads run and follow its steps. Hosts, containers and clusters then show up in the host list, the Kubernetes pages and the Catalog.',
  },
};

/** Logs → Add a Log Source and Infrastructure → Add Resources: a category, then a source, then its steps. */
export function SetupGuidePage({ kind }: { kind: GuideKind }) {
  const [params, setParams] = useSearchParams();
  const sources = sourcesFor(kind);
  const categories = categoriesFor(kind);
  const selected = sources.find((s) => s.id === params.get('source'));
  const category = selected?.category ?? categories.find((c) => c.name === params.get('category'))?.name;
  const values = { intake: intakeUrl(), apiKey: '<API_KEY>' };
  const titles = TITLES[kind];
  return (
    <div className="setup-page">
      <header className="product-header">
        <div className="product-title">
          <PackagePlus size={20} /> {titles.title}
        </div>
      </header>
      <div className="setup-body">
        <nav className="setup-categories">
          <h2>{titles.question}</h2>
          {categories.map((c) => {
            const Icon = ICONS[c.name];
            return (
              <button key={c.name} className={`setup-category-card${category === c.name ? ' active' : ''}`} onClick={() => setParams({ category: c.name })}>
                <Icon size={32} strokeWidth={1.5} className="brand-icon" />
                <span>
                  <strong>{c.name}</strong>
                  <span className="muted">{c.examples}</span>
                </span>
              </button>
            );
          })}
        </nav>
        <section className="setup-detail">
          {selected ? (
            <div className="setup-guide">
              <button className="link-btn" onClick={() => setParams({ category: selected.category })}>
                <ArrowLeft size={14} /> {selected.category}
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
            </div>
          ) : category ? (
            <div className="setup-tiles">
              {sources
                .filter((s) => s.category === category)
                .map((s) => (
                  <button key={s.id} className="setup-tile" onClick={() => setParams({ source: s.id })}>
                    <strong>{s.name}</strong>
                    <span className="muted">{s.summary}</span>
                  </button>
                ))}
            </div>
          ) : (
            <div className="setup-start">
              <PackagePlus size={48} strokeWidth={1.25} className="brand-icon" />
              <h2>{titles.start}</h2>
              <p>{titles.intro}</p>
            </div>
          )}
        </section>
      </div>
    </div>
  );
}
