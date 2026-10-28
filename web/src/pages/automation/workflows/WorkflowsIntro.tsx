import type { ReactNode } from 'react';
import { Link } from 'react-router-dom';
import { ArrowRight, Bell, CalendarClock, GitBranch, Globe, Play, Siren, TriangleAlert, Workflow as WorkflowIcon } from 'lucide-react';
import { IntegrationLogo } from '../../integrations/IntegrationLogo';
import './workflows.css';

/** Tiles floating around the headline: [logo or icon, left %, top px, tilt deg, size]. */
const TILES: [string, number, number, number, 'small' | '' | 'faded'][] = [
  ['slack', 6, 40, -12, ''],
  ['github', 14, 170, 9, 'small'],
  ['pagerduty', 4, 290, -6, 'faded'],
  ['jira', 22, 60, 14, 'small'],
  ['kubernetes', 82, 46, 10, ''],
  ['aws', 92, 160, -9, 'small'],
  ['teams', 76, 250, 7, 'faded'],
  ['opsgenie', 88, 330, -14, 'small'],
  ['zendesk', 18, 330, 6, 'small'],
  ['monitor', 80, 380, -5, 'small'],
];

function Tile({ logo }: { logo: string }) {
  if (logo === 'monitor') return <TriangleAlert size={20} />;
  return <IntegrationLogo logo={logo} name={logo} size={26} />;
}

/** Our drawing of the editor: a title bar and a small flow on a dotted canvas. */
export function CanvasIllustration() {
  const node = (icon: ReactNode, title: string, sub: string, tone: string) => (
    <div className="wfi-node">
      <span className={`wf-tile wf-tone-${tone}`}>{icon}</span>
      <div>
        <div className="wfi-node-title">{title}</div>
        <div className="wfi-node-sub">{sub}</div>
      </div>
    </div>
  );
  return (
    <div className="wfi-frame" aria-hidden="true">
      <div className="wfi-titlebar">
        <WorkflowIcon size={15} />
        <span className="wfi-title">Run Service Restart Pipeline Failover</span>
        <span className="wf-chip wf-chip-published">Published</span>
        <span className="wfi-spacer" />
        <span className="wfi-run">
          <Play size={12} /> Run
        </span>
      </div>
      <div className="wfi-canvas">
        {node(<TriangleAlert size={16} />, 'Monitor trigger', '@workflow-restart-order-service', 'orange')}
        <div className="wfi-line" />
        {node(<CalendarClock size={16} />, 'Check if monitor still in alert state', 'avg error rate, last 5 minutes', 'purple')}
        <div className="wfi-line" />
        {node(<GitBranch size={16} />, 'Still above threshold?', 'Steps.check.output.value > 5', 'teal')}
        <div className="wfi-branches">
          <div className="wfi-branch">
            <span className="wfi-label">true</span>
            {node(<Globe size={16} />, 'Run service restart pipeline', 'POST deploy.example.com', 'blue')}
            <div className="wfi-line short" />
            {node(<Bell size={16} />, 'Tell #checkout', '@slack-checkout', 'orange')}
          </div>
          <div className="wfi-branch">
            <span className="wfi-label">false</span>
            {node(<Siren size={16} />, 'Add a note to the incident', 'Recovered on its own', 'red')}
          </div>
        </div>
      </div>
    </div>
  );
}

/** Workflow Automation → Intro. */
export function WorkflowsIntro() {
  return (
    <div className="wfi-page">
      <section className="au-hero wfi-hero">
        {TILES.map(([logo, left, top, tilt, size]) => (
          <span key={logo} className={`au-mosaic-tile ${size}`} style={{ left: `${left}%`, top, transform: `rotate(${tilt}deg)` }}>
            <Tile logo={logo} />
          </span>
        ))}
        <div className="au-hero-kicker">
          <span className="au-hero-kicker-icon">
            <WorkflowIcon size={16} />
          </span>
          Workflow Automation
        </div>
        <h1>
          Respond to alerts in real time,
          <br />
          end to end and hands-free
        </h1>
        <p>Chain checks, remediations and notifications that start on a monitor, a schedule, an incident or an API call — and see every run, step by step.</p>
        <Link to="/automation/workflows/new" className="au-cta">
          Create Your First Workflow <ArrowRight size={16} />
        </Link>
        <div className="wfi-shot">
          <CanvasIllustration />
        </div>
      </section>
      <section className="wfi-points">
        {[
          ['Start from what happens', 'Monitors mention @workflow-<handle>, cron schedules, incidents, apps and the API start runs.'],
          ['Act across your tools', 'HTTP requests with stored secrets, notifications, incidents, on-call pages, metric and log checks.'],
          ['Decide as it runs', 'If/else branches, loops over lists and waits, reading earlier results with {{ Steps.… }}.'],
        ].map(([t, d]) => (
          <div key={t} className="wfi-point">
            <h3>{t}</h3>
            <p>{d}</p>
          </div>
        ))}
      </section>
    </div>
  );
}
