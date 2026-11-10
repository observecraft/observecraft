import { Link } from 'react-router-dom';
import { MapPin } from 'lucide-react';
import type { SyntheticLocation } from '../../api/synthetics/client';
import { FREQUENCIES, type TestDraft } from '../../api/synthetics/logic';
import { Step } from '../monitors/MonitorEditor';
import { PRIORITY_LABEL, RecipientIcon, recipientLabel } from '../monitors/common';
import { messageRecipients } from '../monitors/message';

/**
 * The editor steps API and browser tests share, numbered from `first`: locations, retries, scheduling and alert
 * conditions (browser tests run every 5 minutes at most: `minFrequency`), downtimes and the monitor.
 */
export function CommonSteps({ first, d, set, location, minFrequency = 0 }: { first: number; d: TestDraft; set: (p: Partial<TestDraft>) => void; location?: SyntheticLocation; minFrequency?: number }) {
  return (
    <>
      <Step n={first} title="Select Locations" done>
        <label className="syn-location">
          <input type="checkbox" className="checkbox" checked disabled />
          <MapPin size={16} />
          <span>
            <b>{location?.name ?? 'ObserveCraft server'}</b>
            <span className="muted">
              Tests run from the ObserveCraft server itself.{' '}
              {location?.privateTargets ? 'This server may reach private addresses.' : 'Private, loopback and link-local addresses are refused (OC_SYNTHETICS_ALLOW_PRIVATE).'}
            </span>
          </span>
        </label>
      </Step>
      <Step n={first + 1} title="Specify Test Retry Conditions" done>
        <div className="mon-eval-row">
          <label>
            Retry test
            <select className="syn-select" value={d.retry.count} onChange={(e) => set({ retry: { ...d.retry, count: Number(e.target.value) } })} aria-label="Retries">
              {[0, 1, 2, 3].map((n) => (
                <option key={n} value={n}>
                  {n}
                </option>
              ))}
            </select>
            times after
          </label>
          <input
            className="mon-num"
            value={d.retry.intervalMs}
            onChange={(e) => set({ retry: { ...d.retry, intervalMs: Math.max(0, Math.min(60_000, Number(e.target.value) || 0)) } })}
            inputMode="numeric"
            aria-label="Retry interval"
            disabled={d.retry.count === 0}
          />
          <label>ms in case of failure</label>
        </div>
      </Step>
      <Step n={first + 2} title="Define Scheduling and Alert Conditions" done>
        <div className="syn-sched">
          <span>Run the test every</span>
          <span className="btn-group" role="group" aria-label="Frequency">
            {FREQUENCIES.filter((f) => f.seconds >= minFrequency).map((f) => (
              <button key={f.seconds} className={`btn${d.frequencySeconds === f.seconds ? ' btn-active' : ''}`} onClick={() => set({ frequencySeconds: f.seconds })}>
                {f.label}
              </button>
            ))}
          </span>
        </div>
        <div className="mon-eval-row">
          <label>An alert is triggered if the test fails for</label>
          <input
            className="mon-num"
            value={d.minFailureDurationMinutes}
            onChange={(e) => set({ minFailureDurationMinutes: Math.max(0, Math.min(120, Number(e.target.value) || 0)) })}
            inputMode="numeric"
            aria-label="Minutes failing"
          />
          <label>
            minutes from
            <select className="syn-select" value={d.minLocationFailed} disabled aria-label="Locations failing">
              <option value={1}>1</option>
            </select>
            of 1 location
          </label>
        </div>
        <label className="mon-check syn-pause">
          <input type="checkbox" className="checkbox" checked={d.paused} onChange={() => set({ paused: !d.paused })} />
          <span className="mon-check-label">Paused: no runs and no alerts until resumed</span>
        </label>
      </Step>
      <Step n={first + 3} title="Downtimes" done>
        <p className="muted syn-text">
          Downtimes from <Link to="/monitors/downtimes">Monitors → Downtimes</Link> silence this test's notifications when they target its monitor or all of its tags. The test keeps running and
          recording results during them.
        </p>
      </Step>
      <Step n={first + 4} title="Configure the Monitor" done={!!d.message.trim()} last>
        <p className="muted syn-text">
          The test alerts through its own monitor, <b>[Synthetics] {d.name || 'name'}</b>. Mention <code>@webhook-…</code>, <code>@slack-…</code> or email addresses to notify;{' '}
          <code>{'{{#is_alert}}'}</code> and <code>{'{{#is_recovery}}'}</code> blocks pick the text per transition. What failed is added to alert notifications.
        </p>
        <textarea className="syn-textarea" rows={5} value={d.message} onChange={(e) => set({ message: e.target.value })} aria-label="Notification message" />
        <div className="syn-row">
          {messageRecipients(d.message).map((r) => (
            <span key={r} className="syn-chip">
              <RecipientIcon r={r} size={13} /> {recipientLabel(r)}
            </span>
          ))}
          <label className="syn-priority">
            Priority
            <select className="syn-select" value={d.priority ?? 0} onChange={(e) => set({ priority: Number(e.target.value) || null })}>
              {['0', '1', '2', '3', '4', '5'].map((p) => (
                <option key={p} value={p}>
                  {PRIORITY_LABEL[p]}
                </option>
              ))}
            </select>
          </label>
        </div>
      </Step>
    </>
  );
}
