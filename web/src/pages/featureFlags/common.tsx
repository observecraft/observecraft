import { useState, type KeyboardEvent } from 'react';
import { Link, NavLink, useNavigate } from 'react-router-dom';
import { ChevronDown, Flag, Layers, Plus, Settings, X } from 'lucide-react';
import type { FlagType } from '../../api/featureFlags/client';
import { TYPE_LABEL, ofrepSnippets } from '../../api/featureFlags/logic';
import { usePopover } from '../../components/ui/Dropdown';
import { CopyButton } from '../settings/SecuritySettings';
import { fill } from '../setup/sources';

/** Where SDKs reach this install's intake. */
export const intakeUrl = () => `${window.location.protocol}//${window.location.hostname}:4318`;

const TABS = [
  { to: '/feature-flags', label: 'Overview', end: true },
  { to: '/feature-flags/history', label: 'Global Flag History' },
];

/** Product header: title with its menu, the section tabs and New Flag. */
export function FlagsHeader() {
  const navigate = useNavigate();
  const { open, setOpen, ref } = usePopover();
  return (
    <header className="product-header">
      <div className="ff-title-menu" ref={ref}>
        <button className="ff-title-btn" onClick={() => setOpen((o) => !o)} aria-haspopup="menu" aria-expanded={open}>
          <Flag size={20} /> Feature Flags <ChevronDown size={16} className="ff-caret" />
        </button>
        {open && (
          <div className="ff-title-pop" role="menu">
            <Link to="/feature-flags" onClick={() => setOpen(false)}>
              <Flag size={15} /> Feature Flags
            </Link>
            <Link to="/feature-flags/environments" onClick={() => setOpen(false)}>
              <Layers size={15} /> Environments
            </Link>
            <Link to="/feature-flags/environments" onClick={() => setOpen(false)}>
              <Settings size={15} /> Settings
            </Link>
          </div>
        )}
      </div>
      <nav className="product-tabs">
        {TABS.map((t) => (
          <NavLink key={t.to} to={t.to} end={t.end} className={({ isActive }) => `product-tab${isActive ? ' active' : ''}`}>
            {t.label}
          </NavLink>
        ))}
      </nav>
      <div className="ff-header-actions">
        <button className="btn btn-primary" onClick={() => navigate('/feature-flags/create')}>
          <Plus size={15} /> New Flag
        </button>
      </div>
    </header>
  );
}

export const TypeBadge = ({ type }: { type: FlagType }) => <span className="ff-type">{TYPE_LABEL[type]}</span>;

export const Swatch = ({ color }: { color: string }) => <span className="ff-swatch" style={{ background: color }} aria-hidden />;

export function EnvPill({ name, enabled }: { name: string; enabled: boolean }) {
  return (
    <span className="ff-env-pill" title={`${enabled ? 'On' : 'Off'} in ${name}`}>
      <span className={`ff-dot${enabled ? ' on' : ''}`} /> {name}
    </span>
  );
}

/** An on/off switch. */
export function Switch({ checked, onChange, label, disabled }: { checked: boolean; onChange: (v: boolean) => void; label: string; disabled?: boolean }) {
  return (
    <label className="ff-switch">
      <input type="checkbox" role="switch" checked={checked} disabled={disabled} onChange={(e) => onChange(e.target.checked)} />
      <span className="ff-switch-track" />
      {label}
    </label>
  );
}

/** Free-text values as chips (Enter or comma adds one), with optional suggestions. */
export function ChipsInput({
  values,
  onChange,
  placeholder,
  suggestions = [],
  label,
}: {
  values: string[];
  onChange: (v: string[]) => void;
  placeholder: string;
  suggestions?: string[];
  label: string;
}) {
  const [text, setText] = useState('');
  const add = (raw: string) => {
    const v = raw.trim();
    if (v && !values.includes(v)) onChange([...values, v]);
    setText('');
  };
  const onKey = (e: KeyboardEvent<HTMLInputElement>) => {
    if (e.key === 'Enter' || e.key === ',') {
      e.preventDefault();
      add(text);
    } else if (e.key === 'Backspace' && !text && values.length) onChange(values.slice(0, -1));
  };
  const listId = `ff-suggest-${label.replace(/\W+/g, '-')}`;
  return (
    <div className="ff-chips">
      {values.map((v) => (
        <span key={v} className="ff-chip">
          {v}
          <button type="button" onClick={() => onChange(values.filter((x) => x !== v))} aria-label={`Remove ${v}`}>
            <X size={12} />
          </button>
        </span>
      ))}
      <input
        aria-label={label}
        list={suggestions.length ? listId : undefined}
        value={text}
        placeholder={values.length ? '' : placeholder}
        onChange={(e) => setText(e.target.value)}
        onKeyDown={onKey}
        onBlur={() => text && add(text)}
      />
      {suggestions.length > 0 && (
        <datalist id={listId}>
          {suggestions
            .filter((s) => !values.includes(s))
            .map((s) => (
              <option key={s} value={s} />
            ))}
        </datalist>
      )}
    </div>
  );
}

/** How an application evaluates a flag with OpenFeature through this install (OFREP). */
export function SetupSnippets({ flagKey, type }: { flagKey: string; type: FlagType }) {
  const snippets = ofrepSnippets(flagKey, type);
  const [id, setId] = useState(snippets[0].id);
  const chosen = snippets.find((s) => s.id === id) ?? snippets[0];
  const code = fill(chosen.code, { intake: intakeUrl(), apiKey: '<API_KEY>' });
  return (
    <div className="ff-setup">
      <div className="ff-sdk-seg" role="tablist" aria-label="SDK">
        {snippets.map((s) => (
          <button key={s.id} role="tab" aria-selected={s.id === id} className={s.id === id ? 'active' : ''} onClick={() => setId(s.id)}>
            {s.label}
          </button>
        ))}
      </div>
      <p className="ff-hint">{chosen.note}</p>
      <div className="ff-snippet">
        <CopyButton text={code} />
        <pre>{code}</pre>
      </div>
      <p className="ff-hint">
        Use an ingest API key from <Link to="/organization-settings/api-keys">Organization Settings → API Keys</Link>. The context’s <code>env</code> picks the environment; without it, the key’s
        default environment applies.
      </p>
    </div>
  );
}

/** First-run illustration: a flag's targeting, its rollout and evaluations (drawn here, our colours). */
export function HeroArt() {
  return (
    <svg className="ff-hero-art" viewBox="0 0 560 330" role="img" aria-label="A feature flag with targeting rules, a rollout at 40% and its evaluations">
      <defs>
        <linearGradient id="ff-art-bar" x1="0" x2="1">
          <stop offset="0" stopColor="#0b6e69" />
          <stop offset="1" stopColor="#2a9d8f" />
        </linearGradient>
      </defs>
      {/* Targeting card */}
      <rect x="16" y="14" width="330" height="222" rx="12" fill="#ffffff" />
      <rect x="36" y="34" width="22" height="22" rx="11" fill="#e3f2ee" />
      <path d="M43 50 V40 h9 l-2 3 l2 3 h-9" fill="none" stroke="#0b6e69" strokeWidth="1.8" strokeLinejoin="round" />
      <rect x="68" y="38" width="130" height="10" rx="5" fill="#1c2333" opacity="0.85" />
      <rect x="276" y="35" width="50" height="20" rx="10" fill="#2ea86b" />
      <circle cx="315" cy="45" r="7" fill="#ffffff" />
      {[0, 1, 2].map((i) => (
        <g key={i} transform={`translate(36 ${76 + i * 50})`}>
          <rect width="290" height="38" rx="8" fill="#f6f7f9" stroke="#e1e3ea" />
          <circle cx="18" cy="19" r="9" fill="#eef0f4" />
          <text x="18" y="23" textAnchor="middle" fontSize="11" fill="#5a6273" fontFamily="sans-serif">
            {i + 1}
          </text>
          <rect x="36" y="11" width={[64, 52, 80][i]} height="7" rx="3.5" fill="#8b93a3" />
          <rect x="36" y="23" width={[110, 90, 70][i]} height="6" rx="3" fill="#c8ccd6" />
          <rect x="204" y="10" width="72" height="18" rx="9" fill={['#dcebfb', '#fdefd6', '#e3f2ee'][i]} />
          <rect x="214" y="16" width="10" height="6" rx="2" fill={['#3d7fe0', '#f2a93b', '#29a8a0'][i]} />
          <rect x="229" y="16" width="38" height="6" rx="3" fill={['#1d5aa6', '#9a5b06', '#073f3d'][i]} opacity="0.7" />
        </g>
      ))}
      {/* Rollout card */}
      <rect x="236" y="196" width="306" height="118" rx="12" fill="#ffffff" />
      <rect x="256" y="214" width="96" height="9" rx="4.5" fill="#1c2333" opacity="0.85" />
      <rect x="470" y="212" width="52" height="14" rx="7" fill="#e3f2ee" />
      <text x="496" y="223" textAnchor="middle" fontSize="10" fill="#073f3d" fontFamily="sans-serif" fontWeight="600">
        40%
      </text>
      <rect x="256" y="238" width="266" height="10" rx="5" fill="#eef0f4" />
      <rect x="256" y="238" width="106" height="10" rx="5" fill="url(#ff-art-bar)" />
      {[0, 1, 2, 3].map((i) => (
        <g key={i}>
          <circle cx={262 + i * 84} cy="270" r="6" fill={i < 2 ? '#0b6e69' : '#ffffff'} stroke="#0b6e69" strokeWidth="2" />
          <rect x={250 + i * 84} y="284" width="26" height="6" rx="3" fill="#c8ccd6" />
          {i < 3 && <line x1={268 + i * 84} y1="270" x2={340 + i * 84} y2="270" stroke={i < 1 ? '#0b6e69' : '#c8ccd6'} strokeWidth="2" strokeDasharray={i < 1 ? '' : '4 4'} />}
        </g>
      ))}
      {/* Evaluations card */}
      <rect x="370" y="14" width="172" height="164" rx="12" fill="#ffffff" />
      <rect x="388" y="32" width="70" height="8" rx="4" fill="#1c2333" opacity="0.85" />
      {[38, 52, 46, 64, 58, 76, 70, 88, 82, 96].map((h, i) => (
        <g key={i}>
          <rect x={390 + i * 14} y={158 - h} width="9" height={h * 0.6} rx="2" fill="#3d7fe0" />
          <rect x={390 + i * 14} y={158 - h * 0.4} width="9" height={h * 0.4} rx="2" fill="#f2a93b" />
        </g>
      ))}
      <line x1="386" y1="160" x2="528" y2="160" stroke="#e1e3ea" />
    </svg>
  );
}
