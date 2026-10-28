import { Link } from 'react-router-dom';
import { Settings } from 'lucide-react';
import { ConfigLayout, ConfigNotAvailable, type ConfigNavGroup } from '../../../components/shell/ConfigLayout';
import './eventSettings.css';

const NAV: ConfigNavGroup[] = [
  {
    section: 'INBOX',
    items: [
      { to: '/event/settings/projects', label: 'Projects' },
      { to: '', label: 'Event Management', heading: true },
      { to: '/event/settings/correlators', label: 'Define Correlators', nested: true },
      { to: '/event/settings/intelligent-correlation', label: 'Intelligent Correlation', nested: true },
      { to: '/event/settings/attributes', label: 'Work Item Attributes' },
      { to: '/event/settings/maintenance-windows', label: 'Maintenance Windows' },
    ],
  },
  {
    section: 'EVENTS',
    items: [
      { to: '/event/settings/pipelines', label: 'Pipelines' },
      { to: '/event/settings/generate-metrics', label: 'Generate Metrics' },
    ],
  },
];

/** Settings › Event Management: correlation, work item attributes and maintenance windows of the Triage Inbox. */
export function EventSettingsLayout() {
  return <ConfigLayout root={{ to: '/event/settings', label: 'Settings', icon: <Settings size={22} /> }} title="Event Management" nav={NAV} label="Event Management settings" />;
}

export function EventProjectsPage() {
  return (
    <ConfigNotAvailable title="Projects">
      Projects group work items for a team. They aren't available yet: the <Link to="/event/inbox">Triage Inbox</Link> shows every work item of the organization.
    </ConfigNotAvailable>
  );
}

export function IntelligentCorrelationPage() {
  return (
    <ConfigNotAvailable title="Intelligent Correlation">
      Automatic, learned correlation isn't available yet. <Link to="/event/settings/correlators">Define correlators</Link> to group related events into work items.
    </ConfigNotAvailable>
  );
}

export function EventPipelinesPage() {
  return (
    <ConfigNotAvailable title="Pipelines">
      Processing pipelines for events aren't available yet: events are stored as the Datadog Agent and the events API send them. Log processing lives in{' '}
      <Link to="/logs/pipelines">Logs › Pipelines</Link>.
    </ConfigNotAvailable>
  );
}

export function EventGenerateMetricsPage() {
  return (
    <ConfigNotAvailable title="Generate Metrics">
      Metrics generated from events aren't available yet. Logs-based metrics are in <Link to="/logs/generate-metrics">Logs › Generate Metrics</Link>.
    </ConfigNotAvailable>
  );
}
