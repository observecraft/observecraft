import { useState } from 'react';
import { useNavigate } from 'react-router-dom';
import { ChevronDown, Clock, Monitor, X } from 'lucide-react';
import { useQuery } from '@tanstack/react-query';
import { mockUser } from '../../../api/dashboards/client';
import { getProfile } from '../../../api/identity/client';
import { API_MODE } from '../../../api/http';
import type { LayoutType } from '../../../api/dashboards/types';
import { usePopover } from '../../../components/ui/Dropdown';
import { Modal } from '../../../components/ui/Modal';
import { useCreateDashboard } from '../queries';

const TEAMS = ['storefront', 'billing', 'platform', 'payments', 'checkout'];

function defaultName(name: string, d: Date): string {
  const first = name.split(' ')[0].toUpperCase();
  return `${first}'s Dashboard ${d.toLocaleDateString('en-US', { weekday: 'short', month: 'short', day: 'numeric' })}, ${d.toLocaleTimeString('en-US')}`;
}

function TeamPicker({ value, onChange }: { value: string[]; onChange: (v: string[]) => void }) {
  const { open, setOpen, ref } = usePopover();
  return (
    <div className="dd cd-teams" ref={ref}>
      <button type="button" className="dd-trigger" onClick={() => setOpen((o) => !o)}>
        <span className={`dd-trigger-label${value.length ? '' : ' muted-text'}`}>{value.length ? value.join(', ') : 'Select up to 5 Teams'}</span>
        <ChevronDown size={14} className="dd-caret" />
      </button>
      {open && (
        <div className="dd-menu dd-left options-menu-list">
          {TEAMS.map((t) => (
            <label key={t}>
              <input
                type="checkbox"
                className="checkbox"
                checked={value.includes(t)}
                disabled={!value.includes(t) && value.length >= 5}
                onChange={() => onChange(value.includes(t) ? value.filter((x) => x !== t) : [...value, t])}
              />
              {t}
            </label>
          ))}
        </div>
      )}
    </div>
  );
}

/** Grid layout diagram (main card). */
function GridArt() {
  return (
    <svg viewBox="0 0 280 120" width="280" height="120" aria-hidden>
      <rect x="2" y="2" width="174" height="108" rx="2" fill="#efe9ff" stroke="#b9a6ff" strokeWidth="2" />
      <rect x="186" y="2" width="38" height="18" rx="2" fill="#efe9ff" stroke="#b9a6ff" strokeWidth="2" />
      <rect x="186" y="28" width="38" height="18" rx="2" fill="#efe9ff" stroke="#b9a6ff" strokeWidth="2" />
      <rect x="232" y="2" width="38" height="46" rx="2" fill="#efe9ff" stroke="#b9a6ff" strokeWidth="2" />
      <rect x="186" y="56" width="84" height="54" rx="2" fill="#efe9ff" stroke="#b9a6ff" strokeWidth="2" />
    </svg>
  );
}

function MiniArt({ variant }: { variant: 'auto' | 'free' }) {
  return (
    <svg viewBox="0 0 140 56" width="140" height="56" aria-hidden>
      {variant === 'auto'
        ? [0, 1].flatMap((r) => [0, 1, 2].map((c) => <rect key={`${r}${c}`} x={2 + c * 46} y={2 + r * 28} width="42" height="24" fill="none" stroke="#c8ccd6" strokeWidth="1.5" />))
        : [
            [2, 2, 60, 10],
            [66, 2, 40, 10],
            [110, 2, 28, 10],
            [2, 16, 30, 14],
            [36, 16, 50, 14],
            [90, 16, 22, 30],
            [2, 34, 84, 12],
          ].map(([x, y, w, h], i) => <rect key={i} x={x} y={y} width={w} height={h} fill="none" stroke="#c8ccd6" strokeWidth="1.5" />)}
    </svg>
  );
}

export function CreateDashboardModal({ onClose }: { onClose: () => void }) {
  const navigate = useNavigate();
  const create = useCreateDashboard();
  // Same key as the top bar's profile query, so it is usually cached already.
  const me = useQuery({ queryKey: ['identity', 'profile'], queryFn: getProfile, enabled: API_MODE === 'live', staleTime: 60_000 });
  const userName = API_MODE === 'live' ? (me.data?.name ?? 'My') : mockUser().name;
  const [openedAt] = useState(() => new Date());
  const [name, setName] = useState<string | null>(null);
  const title = name ?? defaultName(userName, openedAt);
  const [teams, setTeams] = useState<string[]>([]);

  const go = (layoutType: LayoutType) => create.mutate({ title: title.trim() || defaultName(userName, openedAt), teams, layoutType }, { onSuccess: (d) => navigate(`/dashboards/${d.id}`) });

  return (
    <Modal onClose={onClose} label="Create a Dashboard" width={820} height="auto">
      <div className="cd">
        <header className="cd-header">
          <h3>Create a Dashboard</h3>
          <button className="icon-plain" onClick={onClose} title="Close">
            <X size={20} />
          </button>
        </header>
        <div className="cd-fields">
          <label className="cd-name">
            <span>Dashboard Name</span>
            <input autoFocus value={title} onChange={(e) => setName(e.target.value)} onFocus={(e) => e.target.select()} />
          </label>
          <label>
            <span>Team</span>
            <TeamPicker value={teams} onChange={setTeams} />
          </label>
        </div>
        <div className="cd-options">
          <div className="cd-main">
            <GridArt />
            <p>Snap widgets into place on a grid</p>
            <button className="btn btn-primary cd-btn" disabled={create.isPending} onClick={() => go('grid')}>
              New Dashboard
            </button>
          </div>
          <div className="cd-side">
            <div className="cd-alt">
              <div className="cd-alt-top">
                <MiniArt variant="auto" />
                <span>Automatic layout that fits your browser</span>
              </div>
              <button className="btn cd-btn" disabled={create.isPending} onClick={() => go('auto')}>
                <Clock size={15} /> New Timeboard
              </button>
            </div>
            <div className="cd-alt">
              <div className="cd-alt-top">
                <MiniArt variant="free" />
                <span>Pixel-level precision on a scrollable canvas</span>
              </div>
              <button className="btn cd-btn" disabled={create.isPending} onClick={() => go('free')}>
                <Monitor size={15} /> New Screenboard
              </button>
            </div>
          </div>
        </div>
      </div>
    </Modal>
  );
}
