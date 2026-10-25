import { useEffect, useState } from 'react';
import { Link, useNavigate, useSearchParams } from 'react-router-dom';
import {
  Activity,
  ArrowRight,
  ArrowUp,
  BarChart3,
  Bug,
  ChevronRight,
  Cloud,
  Database,
  FileSearch,
  Gauge,
  GitBranch,
  Grid3x3,
  Hexagon,
  LineChart,
  Merge,
  ScrollText,
  Sparkles,
  TrendingUp,
  Waves,
  X,
  type LucideIcon,
} from 'lucide-react';
import type { MonitorInput } from '../../api/monitors/client';
import { Modal } from '../../components/ui/Modal';
import { useSaveMonitor } from './queries';
import { suggestTemplates, type MonitorTemplate } from './templates';
import './monitors.css';

interface TypeEntry {
  label: string;
  icon: LucideIcon;
  /** Editor route; absent = not available yet. */
  to?: string;
  isNew?: boolean;
}

const TYPES: TypeEntry[] = [
  { label: 'Metric', icon: LineChart, to: '/monitors/create/metric' },
  {
    label: 'Change',
    icon: TrendingUp,
    to: '/monitors/create/metric?detection=change',
  },
  { label: 'Anomaly', icon: Waves },
  { label: 'Outlier', icon: Grid3x3 },
  { label: 'Forecast', icon: BarChart3 },
  {
    label: 'Host',
    icon: Hexagon,
    to: '/monitors/create/metric?template=cpu-high',
  },
  { label: 'APM', icon: Activity, to: '/monitors/create/apm' },
  { label: 'Logs', icon: ScrollText, to: '/monitors/create/log' },
  { label: 'CI & Tests', icon: GitBranch },
  { label: 'Cloud Costs', icon: Cloud },
  { label: 'Composite', icon: Merge },
  { label: 'Data Observability', icon: FileSearch },
  { label: 'Database Monitoring', icon: Database },
  { label: 'Error Tracking', icon: Bug },
];

export function NewMonitorPage() {
  const navigate = useNavigate();
  const [params] = useSearchParams();
  const [prompt, setPrompt] = useState('');
  const [asked, setAsked] = useState<string | null>(null);
  const [importing, setImporting] = useState(params.get('import') === '1');
  const suggestions = asked ? suggestTemplates(asked) : [];
  useEffect(() => setImporting(params.get('import') === '1'), [params]);
  const use = (t: MonitorTemplate) => navigate(`/monitors/create/${t.type}?template=${t.id}`);

  return (
    <div className="mon-create">
      <header className="mon-crumbs">
        <Link to="/monitors" className="mon-crumb-root">
          <Gauge size={22} /> Monitors
        </Link>
        <ChevronRight size={18} className="muted" />
        <span>New Monitor</span>
        <span className="mon-crumbs-right">
          <Link className="btn" to="/monitors/templates">
            Browse Templates
          </Link>
          <button className="btn" onClick={() => setImporting(true)}>
            Import From JSON
          </button>
        </span>
      </header>
      <div className="mon-create-body">
        <nav className="mon-types">
          <button className="mon-assist-btn" onClick={() => document.getElementById('mon-assist-input')?.focus()}>
            <Sparkles size={18} /> Create with Assist <ArrowRight size={18} />
          </button>
          <div className="mon-types-label">OR SELECT A TYPE</div>
          {TYPES.map((t) =>
            t.to ? (
              <Link key={t.label} to={t.to} className="mon-type">
                <t.icon size={22} strokeWidth={1.5} /> {t.label}
                {t.isNew && <span className="badge-new">NEW</span>}
              </Link>
            ) : (
              <span key={t.label} className="mon-type disabled" title={`${t.label} monitors aren't available yet`}>
                <t.icon size={22} strokeWidth={1.5} /> {t.label}
              </span>
            ),
          )}
        </nav>
        <section className="mon-assist">
          <h2>Create with Assist</h2>
          <p>Describe what you want to watch; Assist suggests a monitor to start from.</p>
          <form
            className="mon-assist-box"
            onSubmit={(e) => {
              e.preventDefault();
              if (prompt.trim()) setAsked(prompt.trim());
            }}
          >
            <textarea
              id="mon-assist-input"
              rows={3}
              placeholder="I want to monitor my core app performance"
              value={prompt}
              onChange={(e) => setPrompt(e.target.value)}
              onKeyDown={(e) => e.key === 'Enter' && !e.shiftKey && (e.preventDefault(), prompt.trim() && setAsked(prompt.trim()))}
            />
            <button className="mon-assist-send" disabled={!prompt.trim()} title="Suggest monitors">
              <ArrowUp size={18} />
            </button>
          </form>
          {asked && (
            <div className="mon-assist-results">
              {suggestions.length === 0 ? (
                <p className="muted">No template matches “{asked}”. Try mentioning what to watch, e.g. CPU, memory, disk, errors, latency or traffic — or pick a type on the left.</p>
              ) : (
                <>
                  <p className="muted">Suggested for “{asked}”:</p>
                  {suggestions.slice(0, 4).map((t) => (
                    <button key={t.id} className="mon-assist-card" onClick={() => use(t)}>
                      <span className="mon-tpl-source">{t.source}</span>
                      <b>{t.title}</b>
                      <span className="muted">{t.description}</span>
                      <code>{t.query}</code>
                    </button>
                  ))}
                </>
              )}
            </div>
          )}
        </section>
      </div>
      {importing && <ImportModal onClose={() => (params.get('import') ? navigate('/monitors/create', { replace: true }) : setImporting(false))} />}
    </div>
  );
}

function ImportModal({ onClose }: { onClose: () => void }) {
  const navigate = useNavigate();
  const save = useSaveMonitor();
  const [text, setText] = useState('');
  const [error, setError] = useState<string | null>(null);
  const submit = () => {
    setError(null);
    let input: MonitorInput;
    try {
      input = JSON.parse(text);
    } catch {
      return setError('That isn’t valid JSON.');
    }
    save.mutate(input, {
      onSuccess: (m) => navigate(`/monitors/${m.id}`),
      onError: (e) => setError((e as Error).message),
    });
  };
  return (
    <Modal onClose={onClose} label="Import monitor" width={680} height="auto">
      <div className="small-modal">
        <div className="modal-title-row">
          <h3>Import Monitor From JSON</h3>
          <button className="icon-plain" onClick={onClose} title="Close">
            <X size={18} />
          </button>
        </div>
        <p className="muted">Paste a monitor exported with “Export Monitor” (name, type, query, options, message, priority, tags, teams).</p>
        <textarea className="mon-json" rows={14} value={text} onChange={(e) => setText(e.target.value)} spellCheck={false} />
        {error && <p className="form-error">{error}</p>}
        <div className="small-modal-actions">
          <button className="btn" onClick={onClose}>
            Cancel
          </button>
          <button className="btn btn-primary" disabled={!text.trim() || save.isPending} onClick={submit}>
            Import
          </button>
        </div>
      </div>
    </Modal>
  );
}
