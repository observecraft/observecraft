import { useEffect, useState } from 'react';
import { createPortal } from 'react-dom';
import { useNavigate } from 'react-router-dom';
import { ChevronDown, PanelRightClose, PanelRightOpen, Siren, X } from 'lucide-react';
import { UserAvatar } from '../../components/ui/UserAvatar';
import { useProfile, useUsers } from '../settings/queries';
import { useDeclare } from './queries';

const TYPES = ['General Incident', 'Production Incident', 'Security Incident', 'EventBased-Alerts'];

/** Lowest to highest, as laid out left to right. */
const LEVELS = [
  { value: 'UNKNOWN', label: 'Unknown', sub: 'Investigation', color: null },
  { value: 'SEV-5', label: 'SEV-5', sub: 'Minor', color: '#f7a58c' },
  { value: 'SEV-4', label: 'SEV-4', sub: 'Low', color: '#f0606f' },
  { value: 'SEV-3', label: 'SEV-3', sub: 'Moderate', color: '#e0457a' },
  { value: 'SEV-2', label: 'SEV-2', sub: 'High', color: '#b3215f' },
  { value: 'SEV-1', label: 'SEV-1', sub: 'Critical', color: '#6d0a33' },
];

/** `initialTitle`/`detection` prefill the form when declaring from another page (e.g. a monitor). */
export function DeclareModal({ onClose, initialTitle = '', detection = 'employee' }: { onClose: () => void; initialTitle?: string; detection?: string }) {
  const navigate = useNavigate();
  const declare = useDeclare();
  const me = useProfile();
  const users = useUsers({ query: '', statuses: ['active'], offset: 0, limit: 200 });
  const [title, setTitle] = useState(initialTitle);
  const [type, setType] = useState(TYPES[0]);
  const [severity, setSeverity] = useState('UNKNOWN');
  // undefined = "me" (default); null = cleared.
  const [commander, setCommander] = useState<string | null | undefined>(undefined);
  const [help, setHelp] = useState(true);
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => e.key === 'Escape' && onClose();
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  }, [onClose]);
  const commanderId = commander === undefined ? me.data?.id ?? null : commander;
  const commanderUser = (users.data?.users ?? []).find((u) => u.id === commanderId) ?? (me.data && me.data.id === commanderId ? me.data : null);

  return createPortal(
    <div className="modal-backdrop" onMouseDown={(e) => e.target === e.currentTarget && onClose()}>
      <div className={`declare${help ? ' with-help' : ''}`} role="dialog" aria-modal="true" aria-label="Declare incident">
        <form
          className="declare-main"
          onSubmit={(e) => {
            e.preventDefault();
            declare.mutate(
              { title, severity, incidentType: type, commanderId: commanderId ?? undefined, detectionMethod: detection },
              { onSuccess: (d) => navigate(`/incidents/${(d as { id: number }).id}`) },
            );
          }}
        >
          <header className="declare-head">
            <Siren size={24} />
            <h2>Declare Incident</h2>
            {!help && (
              <button type="button" className="declare-icon" onClick={() => setHelp(true)} title="Show help">
                <PanelRightOpen size={20} />
              </button>
            )}
            <button type="button" className="declare-icon" onClick={onClose} title="Close (Esc)">
              <X size={24} />
            </button>
          </header>
          <div className="declare-body">
            <label className="declare-label">
              <span>
                Title <span className="req">*</span>
              </span>
              <input className="declare-input" placeholder={'Say what\'s happening. Example: "Login page is not loading"'} value={title} onChange={(e) => setTitle(e.target.value)} maxLength={300} required autoFocus />
            </label>
            <label className="declare-label">
              <span>
                Type <span className="req">*</span>
              </span>
              <span className="declare-select">
                <select value={type} onChange={(e) => setType(e.target.value)}>
                  {TYPES.map((t) => (
                    <option key={t}>{t}</option>
                  ))}
                </select>
                <ChevronDown size={16} />
              </span>
            </label>
            <div className="sev-cards" role="radiogroup" aria-label="Severity">
              {LEVELS.map((l) => (
                <button key={l.value} type="button" role="radio" aria-checked={severity === l.value} className={`sev-card${severity === l.value ? ' active' : ''}`} onClick={() => setSeverity(l.value)}>
                  <span className="sev-name">
                    {l.color && <i style={{ background: l.color }} />}
                    {l.label}
                  </span>
                  <span className="sev-sub">{l.sub}</span>
                </button>
              ))}
            </div>
            <div className="declare-label">
              Incident Commander
              <div className="commander-row">
                {commanderUser ? <UserAvatar id={commanderUser.id} name={commanderUser.name} size={40} /> : <span className="commander-empty" />}
                <span className="declare-select grow">
                  <select value={commanderId ?? ''} onChange={(e) => setCommander(e.target.value || null)}>
                    <option value="">Unassigned</option>
                    {me.data && !(users.data?.users ?? []).some((u) => u.id === me.data.id) && <option value={me.data.id}>{me.data.name.toUpperCase()}</option>}
                    {(users.data?.users ?? []).map((u) => (
                      <option key={u.id} value={u.id}>
                        {u.name.toUpperCase()}
                      </option>
                    ))}
                  </select>
                  {commanderId && (
                    <button type="button" className="commander-clear" onClick={() => setCommander(null)} title="Clear">
                      <X size={12} />
                    </button>
                  )}
                  <ChevronDown size={16} />
                </span>
              </div>
            </div>
            {declare.isError && <p className="form-error">{(declare.error as Error).message}</p>}
          </div>
          <footer className="declare-foot">
            <button type="button" className="btn declare-btn" onClick={onClose}>
              Cancel
            </button>
            <button className="btn btn-primary declare-btn" disabled={!title.trim() || declare.isPending}>
              {declare.isPending ? 'Declaring…' : 'Declare Incident'}
            </button>
          </footer>
        </form>
        {help && (
          <aside className="declare-help">
            <header>
              <h3>Help</h3>
              <button type="button" className="declare-icon dark" onClick={() => setHelp(false)} title="Hide help">
                <PanelRightClose size={20} />
              </button>
            </header>
            <div className="declare-help-body">
              <h4>
                <ChevronDown size={18} /> Declaring an Incident
              </h4>
              <b>Declare an incident if:</b>
              <ul>
                <li>An issue is or may be impacting customers or services.</li>
                <li>You don't know if you should call an incident — declare it, notify other people and adjust the severity as you learn more.</li>
              </ul>
              <b>Recommendations:</b>
              <ul>
                <li>Estimate impact. Severity levels go from SEV-1 (Critical) to SEV-5 (Minor). In case of doubt, choose the higher severity.</li>
                <li>Use Unknown while you are still investigating, and set the severity as soon as you know more.</li>
                <li>Post an update in your team's incident channel, and in your company-wide channel when customers are affected.</li>
                <li>Keep the timeline current: notes and status changes become the post-incident review.</li>
              </ul>
            </div>
          </aside>
        )}
      </div>
    </div>,
    document.body,
  );
}
