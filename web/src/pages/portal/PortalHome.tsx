import { useEffect, useRef, useState, type ReactNode } from 'react';
import { Link, useNavigate } from 'react-router-dom';
import { Bot, Boxes, GitPullRequest, LayoutGrid, PanelLeftClose, PanelLeftOpen, Rocket, Settings2, Ticket } from 'lucide-react';
import { dateLine, firstName, greeting } from '../../api/portal/logic';
import { useProfile } from '../settings/queries';
import { HomeApps } from './HomeApps';
import { HomeEntities } from './HomeEntities';
import { HomeGroup, HomeSection } from './homeCommon';
import { HomePullRequests } from './HomePullRequests';
import { HomeTickets } from './HomeTickets';
import '../integrations/integrations.css';
import './home.css';

const NAV: { id: string; label: string; icon: ReactNode }[] = [
  { id: 'prs', label: 'Your PRs', icon: <GitPullRequest size={16} /> },
  { id: 'tickets', label: 'Your Tickets', icon: <Ticket size={16} /> },
  { id: 'agents', label: 'Your Agents', icon: <Bot size={16} /> },
  { id: 'entities', label: 'Services & Entities', icon: <Boxes size={16} /> },
  { id: 'apps', label: 'Apps', icon: <LayoutGrid size={16} /> },
];

/** Home → Your Agents. No coding agents report here yet, so the groups are honest zeros. */
function HomeAgents() {
  return (
    <HomeSection id="agents" icon={<Bot size={16} />} title="Your Agents" actions={<span className="sc-muted ph-updated">Updated just now</span>}>
      {['Needs attention', 'Active', 'Recently finished'].map((label) => (
        <HomeGroup key={label} label={label} count={0} defaultOpen={false}>
          <div className="ph-group-empty">No agent sessions</div>
        </HomeGroup>
      ))}
    </HomeSection>
  );
}

/** Developer Portal → Home: the member's pull requests, tickets, agents, services and the org's apps. */
export function PortalHome() {
  const navigate = useNavigate();
  const me = useProfile();
  const mainRef = useRef<HTMLDivElement>(null);
  const [active, setActive] = useState('prs');
  const [collapsed, setCollapsed] = useState(false);
  const [now] = useState(() => new Date());
  const profile = me.data;
  const teams = profile?.teams ?? [];
  const isWriter = profile?.roles.some((r) => r.id === `${profile.org.id}-admin` || r.id === `${profile.org.id}-standard`) ?? false;

  // The section nearest the top of the scroll area is the active one.
  useEffect(() => {
    const main = mainRef.current;
    if (!main) return;
    const onScroll = () => {
      const top = main.getBoundingClientRect().top + 90;
      let current = NAV[0].id;
      for (const n of NAV) {
        const el = document.getElementById(n.id);
        if (el && el.getBoundingClientRect().top <= top) current = n.id;
      }
      setActive(current);
    };
    main.addEventListener('scroll', onScroll, { passive: true });
    return () => main.removeEventListener('scroll', onScroll);
  }, []);

  const go = (id: string) => {
    setActive(id);
    document.getElementById(id)?.scrollIntoView({ behavior: 'smooth', block: 'start' });
  };

  return (
    <div className={`ph-page${collapsed ? ' collapsed' : ''}`}>
      <aside className="ph-nav" aria-label="Home sections">
        <nav>
          {NAV.map((n) => (
            <button key={n.id} className={`ph-nav-item${active === n.id ? ' active' : ''}`} onClick={() => go(n.id)} title={collapsed ? n.label : undefined} aria-current={active === n.id}>
              {n.icon}
              <span>{n.label}</span>
            </button>
          ))}
        </nav>
        <div className="ph-nav-bottom">
          <Link to="/portal/settings/sources" className="ph-nav-item" title={collapsed ? 'Get Started' : undefined}>
            <Rocket size={16} />
            <span>Get Started</span>
          </Link>
          <button className="ph-nav-collapse" onClick={() => setCollapsed(!collapsed)} aria-label={collapsed ? 'Expand navigation' : 'Collapse navigation'}>
            {collapsed ? <PanelLeftOpen size={16} /> : <PanelLeftClose size={16} />}
          </button>
        </div>
      </aside>
      <div className="ph-main" ref={mainRef}>
        <div className="ph-inner">
          <header className="ph-hero">
            <div>
              <div className="ph-date">{dateLine(now)}</div>
              <h1>
                {greeting(now)}
                {profile && `, ${firstName(profile.name, profile.email)}`}
              </h1>
            </div>
            <div className="ph-teams">
              <span className="ph-teams-label">Your teams</span>
              {teams.length === 0 ? (
                <span className="sc-muted">None yet</span>
              ) : (
                teams.slice(0, 4).map((t) => (
                  <span key={t.handle} className="ph-team-chip" title={t.name}>
                    @{t.handle}
                  </span>
                ))
              )}
              {teams.length > 4 && <span className="ph-team-chip">+{teams.length - 4}</span>}
              <button className="btn btn-sm" onClick={() => navigate('/teams')}>
                <Settings2 size={13} /> Configure
              </button>
            </div>
          </header>
          <HomePullRequests />
          <HomeTickets />
          <HomeAgents />
          <HomeEntities myTeams={teams.map((t) => t.handle)} />
          <HomeApps canEdit={isWriter} />
        </div>
      </div>
    </div>
  );
}
