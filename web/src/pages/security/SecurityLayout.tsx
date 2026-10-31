import { Outlet } from 'react-router-dom';
import { SecurityHeader } from './common';
import '../monitors/monitors.css';
import './security.css';

/** Security's frame: Overview, Inbox, Signals and Research Feed. */
export function SecurityLayout() {
  return (
    <div className="mon-layout sec-layout">
      <SecurityHeader
        product="Security"
        settings="/security/siem/settings"
        tabs={[
          ['Overview', '/security', true],
          ['Inbox', '/security/inbox'],
          ['Signals', '/security/signals'],
          ['Research Feed', '/security/feed'],
        ]}
      />
      <div className="mon-section">
        <Outlet />
      </div>
    </div>
  );
}

/** Cloud SIEM's frame: Overview, Content Packs, Signals and Detection Rules. */
export function SiemLayout() {
  return (
    <div className="mon-layout sec-layout">
      <SecurityHeader
        product="Cloud SIEM"
        settings="/security/siem/settings"
        tabs={[
          ['Overview', '/security/siem', true],
          ['Content Packs', '/security/siem/content-packs'],
          ['Signals', '/security/siem/signals'],
          ['Detection Rules', '/security/siem/rules'],
        ]}
      />
      <div className="mon-section">
        <Outlet />
      </div>
    </div>
  );
}
