import { useState } from 'react';
import { Bell } from 'lucide-react';
import { UPTIME_DAYS } from '../../api/statusPages/status';
import { OVERALL_STATUS_LABEL, type PublicStatusPage } from '../../api/statusPages/types';
import { ComponentList, NoticeCard, StatusBadge } from './StatusParts';
import './statusPages.css';

const DAY = 86_400_000;
const shortDate = (t: number) => new Date(t).toLocaleDateString(undefined, { timeZone: 'UTC', year: 'numeric', month: 'short', day: 'numeric' });

/** The subscribe button: the page offers updates by email and/or Slack (their delivery isn't built yet). */
function Subscribe({ email, slack }: { email: boolean; slack: boolean }) {
  const [open, setOpen] = useState(false);
  const channels = [email && 'email', slack && 'Slack'].filter(Boolean).join(' or ');
  return (
    <div className="stp-subscribe">
      <button type="button" className="stp-bell" title="Subscribe to updates" onClick={() => setOpen((o) => !o)} aria-expanded={open}>
        <Bell size={16} />
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
 * A status page as its visitors see it: a header band with the overall status, open notices, the
 * components with their uptime over the last 60 days, and (on the page itself) the notice history.
 * Used at /status/<slug> and as the editor's live preview (`preview`: links stay put).
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
    <div className={`stp-view${preview ? ' stp-view-preview' : ''}`}>
      <div className="stp-view-hero">
        <header className="stp-view-head">
          <div className="stp-view-brand">
            {logo ? <img className="stp-logo" src={logo} alt={page.name} onError={() => setBrokenLogo(logo)} /> : page.name && <span className="stp-view-name">{page.name}</span>}
          </div>
          <nav className="stp-view-nav">
            <a href="#history" onClick={preview ? (e) => e.preventDefault() : undefined}>
              History
            </a>
            {(page.emailSubscriptions || page.slackSubscriptions) && <Subscribe email={page.emailSubscriptions} slack={page.slackSubscriptions} />}
          </nav>
        </header>
        <div className="stp-view-status">
          <StatusBadge status={page.overallStatus} size={48} />
          <h1>{OVERALL_STATUS_LABEL[page.overallStatus]}</h1>
        </div>
      </div>

      <div className="stp-view-body">
        {open.length > 0 && (
          <div className="stp-notices">
            {open.map((n) => (
              <NoticeCard key={n.id} notice={n} components={page.components} />
            ))}
          </div>
        )}

        <section className="stp-system">
          <div className="stp-system-head">
            <h2>System Status</h2>
            <span className="stp-system-range">
              <span className="stp-range-chip">{UPTIME_DAYS}d</span>
              {shortDate(today - (UPTIME_DAYS - 1) * DAY)} – {shortDate(today)}
            </span>
          </div>
          <div className="stp-system-body">
            {page.components.length ? (
              <ComponentList components={page.components} notices={page.notices} visualization={page.visualization} />
            ) : (
              <p className="stp-system-empty">No components available at the moment.</p>
            )}
          </div>
        </section>

        {!preview && (
          <section id="history" className="stp-history">
            <h2>History</h2>
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
        )}
      </div>
      <footer className="stp-view-footer">
        Powered by <strong>ObserveCraft</strong>
      </footer>
    </div>
  );
}
