import { useState } from 'react';
import { AlertTriangle, Bell, CheckCircle2, OctagonAlert } from 'lucide-react';
import { UPTIME_DAYS } from '../../api/statusPages/status';
import { OVERALL_STATUS_LABEL, VISIBILITY_LABEL, type ComponentStatus, type PublicStatusPage } from '../../api/statusPages/types';
import { ComponentList, NoticeCard } from './StatusParts';
import './statusPages.css';

const DAY = 86_400_000;
const shortDate = (t: number) => new Date(t).toLocaleDateString(undefined, { timeZone: 'UTC', year: 'numeric', month: 'short', day: 'numeric' });

function StatusIcon({ status }: { status: ComponentStatus }) {
  if (status === 'operational') return <CheckCircle2 size={44} />;
  if (status === 'major_outage') return <OctagonAlert size={44} />;
  return <AlertTriangle size={44} />;
}

/** The subscribe button: the page offers updates by email and/or Slack (their delivery isn't built yet). */
function Subscribe({ email, slack }: { email: boolean; slack: boolean }) {
  const [open, setOpen] = useState(false);
  const channels = [email && 'email', slack && 'Slack'].filter(Boolean).join(' or ');
  return (
    <div className="stp-subscribe">
      <button type="button" className="btn" onClick={() => setOpen((o) => !o)} aria-expanded={open}>
        <Bell size={14} /> Subscribe
      </button>
      {open && (
        <div className="stp-subscribe-pop" role="dialog">
          Updates by {channels} aren&apos;t available on this server yet. Check back here for the latest status.
        </div>
      )}
    </div>
  );
}

/**
 * A status page as its visitors see it: header, overall status, open notices, the components with
 * their uptime over the last 60 days, and the notice history. Used at /status/<slug> and as the
 * editor's live preview (`preview`: the visibility badge shows and the history link stays put).
 */
export function StatusPageView({ page, preview = false }: { page: PublicStatusPage; preview?: boolean }) {
  const now = Date.now();
  const today = Math.floor(now / DAY) * DAY;
  const open = page.notices.filter((n) => n.resolvedAt == null);
  const past = page.notices.filter((n) => n.resolvedAt != null);
  // A logo that fails to load falls back to the title.
  const [brokenLogo, setBrokenLogo] = useState('');
  const logo = page.logoUrl && page.logoUrl !== brokenLogo ? page.logoUrl : '';
  return (
    <div className="stp-view">
      <header className="stp-view-head">
        <div className="stp-view-brand">
          {logo ? <img className="stp-logo" src={logo} alt={page.name} onError={() => setBrokenLogo(logo)} /> : <h1 className="stp-public-title">{page.name || 'Untitled page'}</h1>}
          {preview && <span className="tag-chip">{VISIBILITY_LABEL[page.visibility]}</span>}
        </div>
        <nav className="stp-view-nav">
          <a href="#history" onClick={preview ? (e) => e.preventDefault() : undefined}>
            History
          </a>
          {(page.emailSubscriptions || page.slackSubscriptions) && <Subscribe email={page.emailSubscriptions} slack={page.slackSubscriptions} />}
        </nav>
      </header>
      {page.description && <p className="muted">{page.description}</p>}

      <div className={`stp-headline stp-${page.overallStatus}`}>
        <span className="stp-headline-icon">
          <StatusIcon status={page.overallStatus} />
        </span>
        <span>{OVERALL_STATUS_LABEL[page.overallStatus]}</span>
      </div>

      {open.length > 0 && (
        <div className="stp-notices">
          {open.map((n) => (
            <NoticeCard key={n.id} notice={n} components={page.components} />
          ))}
        </div>
      )}

      <section className="stp-card">
        <div className="stp-card-head">
          <h2>System Status</h2>
          <span className="stp-card-range">
            <span className="tag-chip">{UPTIME_DAYS}d</span>
            <span className="muted">
              {shortDate(today - (UPTIME_DAYS - 1) * DAY)} – {shortDate(today)}
            </span>
          </span>
        </div>
        {page.components.length ? <ComponentList components={page.components} notices={page.notices} /> : <p className="muted stp-card-empty">No components available</p>}
      </section>

      <section id="history">
        <h2 className="stp-view-h2">History</h2>
        {past.length ? (
          <div className="stp-notices">
            {past.map((n) => (
              <NoticeCard key={n.id} notice={n} components={page.components} />
            ))}
          </div>
        ) : (
          <p className="muted">No incidents in the last {UPTIME_DAYS} days.</p>
        )}
      </section>
      <footer className="stp-public-footer muted">Powered by ObserveCraft</footer>
    </div>
  );
}
