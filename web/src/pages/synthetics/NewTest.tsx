import { useState } from 'react';
import { Link, useNavigate } from 'react-router-dom';
import { ArrowLeftToLine, ChevronDown, ChevronRight, Gauge, HeartPulse, List, MonitorSmartphone, MousePointerClick, Network, Plus, Radar, Search, ShieldAlert, Smartphone, X } from 'lucide-react';
import { TEST_TEMPLATES, type TestTemplate } from '../../api/synthetics/logic';
import { Modal } from '../../components/ui/Modal';
import './synthetics.css';
import './syntheticsProduct.css';

const TEST_KINDS = [
  { id: 'api', icon: HeartPulse, title: 'API Test', description: 'Check services, websites and APIs over HTTP, SSL, DNS or TCP on a schedule, and alert when they fail or slow down.' },
  { id: 'multistep', icon: Network, title: 'Multistep API Test', description: 'Chain API requests, passing values from one response to the next request.' },
  { id: 'browser', icon: MousePointerClick, title: 'Browser Test', description: 'Replay recorded steps of a business flow in a browser.' },
  { id: 'mobile', icon: Smartphone, title: 'Mobile Application Test', description: 'Run recorded flows of your iOS and Android applications.' },
  { id: 'network', icon: Radar, title: 'Network Path Test', description: 'Trace the network path to an endpoint, hop by hop, with latency and loss.' },
] as const;

/** New Synthetics Test: the kinds of test; API tests are available, the others are shown as coming. */
export function NewTestModal({ onClose }: { onClose: () => void }) {
  const navigate = useNavigate();
  const [aside, setAside] = useState(true);
  const [open, setOpen] = useState<string | null>('start');
  return (
    <Modal onClose={onClose} label="New Synthetics Test" width={aside ? 1180 : 860} height="auto">
      <div className={`syn-new${aside ? '' : ' no-aside'}`}>
        <div className="syn-new-main">
          <h2>
            New Synthetics Test
            <button className="icon-plain" title="Close" onClick={onClose}>
              <X size={20} />
            </button>
          </h2>
          <div className="syn-new-grid">
            {TEST_KINDS.map((k) => {
              const ready = k.id === 'api';
              return (
                <button
                  key={k.id}
                  className="syn-new-card"
                  disabled={!ready}
                  onClick={() => {
                    onClose();
                    navigate('/synthetics/templates');
                  }}
                >
                  <k.icon size={40} strokeWidth={1.4} />
                  <span>
                    {!ready && <em>NOT AVAILABLE YET</em>}
                    <b>{k.title}</b>
                    <span>{k.description}</span>
                  </span>
                </button>
              );
            })}
          </div>
        </div>
        {aside ? (
          <aside className="syn-new-aside">
            <header>
              <h3>Learn More</h3>
              <button className="icon-plain" title="Hide" onClick={() => setAside(false)}>
                <ArrowLeftToLine size={18} />
              </button>
            </header>
            <div className="syn-new-tip">
              <h4>Start from a template</h4>
              <p>
                API tests come with templates for response time, certificate validity, DNS resolution and security headers. Each one fills in the request and its assertions; change anything before you
                save.
              </p>
            </div>
            {(
              [
                [
                  'start',
                  'Get started with synthetic tests',
                  'Tests run from the ObserveCraft server on their schedule. A failing test alerts through its monitor, so notifications, downtimes and SLOs work as for any monitor, and every run writes synthetics.* metrics for dashboards.',
                ],
                [
                  'vars',
                  'Reuse values with global variables',
                  'Store hosts, tokens and passwords once in Settings → Global Variables and write {{ NAME }} in a test. Secure values are hidden in results.',
                ],
                [
                  'ci',
                  'Run tests from CI',
                  'Continuous Testing from CI pipelines is not available yet. Start a run by hand with Run Test Now, or from the API (POST /api/v1/synthetics/tests/{id}/run).',
                ],
              ] as [string, string, string][]
            ).map(([id, title, text]) => (
              <section key={id} className="syn-new-fold">
                <button onClick={() => setOpen(open === id ? null : id)}>
                  {open === id ? <ChevronDown size={16} /> : <ChevronRight size={16} />} {title}
                </button>
                {open === id && <p>{text}</p>}
              </section>
            ))}
          </aside>
        ) : (
          <button className="syn-new-show" title="Learn more" onClick={() => setAside(true)}>
            <MonitorSmartphone size={16} />
          </button>
        )}
      </div>
    </Modal>
  );
}

const GROUP_ICON = { Availability: Gauge, 'Security Checks': ShieldAlert };

/** New API Test: start from scratch or from a template of the Availability or Security Checks features. */
export function ApiTemplatesPage() {
  const navigate = useNavigate();
  const [search, setSearch] = useState('');
  const [group, setGroup] = useState<TestTemplate['group'] | null>(null);
  const shown = TEST_TEMPLATES.filter((t) => (!group || t.group === group) && `${t.title} ${t.description}`.toLowerCase().includes(search.trim().toLowerCase())).sort(
    (a, b) => Number(!!a.soon) - Number(!!b.soon),
  );
  const use = (t: TestTemplate) => !t.soon && navigate(`/synthetics/create?template=${t.id}`);
  return (
    <div className="mon-editor">
      <header className="mon-crumbs syn-test-crumbs">
        <Link to="/synthetics" className="mon-crumb-root" title="Synthetic Monitoring & Testing">
          <Radar size={20} />
        </Link>
        <span className="muted">/</span>
        <Link to="/synthetics/tests" className="mon-crumb-root" title="Tests">
          <List size={20} />
        </Link>
        <span className="muted">/</span>
        <HeartPulse size={18} />
        <span>New API Test</span>
      </header>
      <div className="syn-templates">
        <nav className="syn-templates-nav">
          <button className="syn-templates-scratch" onClick={() => navigate('/synthetics/create')}>
            <Plus size={16} /> Start from scratch
          </button>
          <h5>FEATURES</h5>
          {(['Availability', 'Security Checks'] as const).map((g) => {
            const Icon = GROUP_ICON[g];
            return (
              <button key={g} className={group === g ? 'active' : ''} onClick={() => setGroup(group === g ? null : g)}>
                <Icon size={16} /> {g}
              </button>
            );
          })}
        </nav>
        <div className="syn-templates-main">
          <div className="syn-searchbar">
            <Search size={16} />
            <input placeholder="Find a template…" value={search} onChange={(e) => setSearch(e.target.value)} aria-label="Find a template" autoFocus />
          </div>
          <div className="syn-templates-grid">
            {!group && !search && (
              <button className="syn-template scratch" onClick={() => navigate('/synthetics/create')}>
                <span className="syn-template-art">
                  <Plus size={28} />
                  <b>Start from Scratch</b>
                  <span>Create a blank API test</span>
                </span>
              </button>
            )}
            {shown.map((t) => {
              const Icon = GROUP_ICON[t.group];
              return (
                <button key={t.id} className={`syn-template${t.soon ? ' soon' : ''}`} disabled={t.soon} onClick={() => use(t)} title={t.soon ? 'This request type is not available yet' : undefined}>
                  <span className={`syn-template-art ${t.group === 'Availability' ? 'availability' : 'security'}`}>
                    <i>
                      <Icon size={22} />
                    </i>
                  </span>
                  <b>
                    {t.title}
                    {t.soon && <span className="syn-soon">soon</span>}
                  </b>
                  <span className="muted">{t.description}</span>
                </button>
              );
            })}
            {shown.length === 0 && <p className="muted">No template matches “{search}”.</p>}
          </div>
        </div>
      </div>
    </div>
  );
}
