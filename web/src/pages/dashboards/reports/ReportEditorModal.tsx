import { useState } from 'react';
import { Info, X } from 'lucide-react';
import { WEEKDAYS, defaultSchedule, describeSchedule, type DashboardReport, type ReportFrequency, type ReportSchedule } from '../../../api/dashboardReports/types';
import type { TemplateVarValues } from '../../../api/dashboards/types';
import { Modal } from '../../../components/ui/Modal';
import { useDashboard } from '../queries';
import { useMe, useReportOptions, useSaveReport } from './queries';
import './reports.css';

const ZONES = ['UTC', 'Europe/London', 'Europe/Paris', 'Europe/Berlin', 'America/New_York', 'America/Chicago', 'America/Los_Angeles', 'Asia/Tokyo', 'Asia/Singapore', 'Australia/Sydney'];

/** Create or edit a report of one dashboard: variables, time frame, schedule, recipients and message. */
export function ReportEditorModal({
  dashboardId,
  report,
  initialValues,
  onClose,
  onSaved,
}: {
  dashboardId: string;
  report?: DashboardReport;
  initialValues?: TemplateVarValues;
  onClose: () => void;
  onSaved?: (r: DashboardReport) => void;
}) {
  const dash = useDashboard(dashboardId);
  const options = useReportOptions();
  const me = useMe();
  const save = useSaveReport();
  const [name, setName] = useState(report?.name ?? '');
  const [values, setValues] = useState<TemplateVarValues>(report?.templateValues ?? initialValues ?? {});
  const [timeFrame, setTimeFrame] = useState(report?.timeFrame ?? '1w');
  const [schedule, setSchedule] = useState<ReportSchedule>(() => report?.schedule ?? { ...defaultSchedule(), timezone: me.data?.timezone || defaultSchedule().timezone });
  const [recipients, setRecipients] = useState<string[]>(report?.recipients ?? []);
  const [draft, setDraft] = useState('');
  const [message, setMessage] = useState(report?.message ?? '');
  const title = dash.data?.title ?? report?.dashboardTitle ?? '';
  const vars = dash.data?.templateVariables ?? [];
  const channels = options.data?.channels ?? [];
  const emailOff = options.data && !options.data.emailEnabled;

  const addRecipient = (raw: string) => {
    const parts = raw
      .split(/[\s,;]+/)
      .map((p) => p.trim().replace(/^@/, ''))
      .filter(Boolean);
    if (parts.length) setRecipients((rs) => [...rs, ...parts.filter((p) => !rs.includes(p.toLowerCase()))]);
    setDraft('');
  };
  const set = (patch: Partial<ReportSchedule>) => setSchedule((s) => ({ ...s, ...patch }));
  const submit = () => {
    const pending = draft.trim() ? [...recipients, draft.trim()] : recipients;
    save.mutate(
      {
        id: report?.id,
        input: {
          name: name.trim() || `${title} report`,
          dashboardId,
          templateValues: Object.fromEntries(vars.map((v) => [v.name, values[v.name] ?? v.default])),
          timeFrame,
          schedule: {
            ...schedule,
            weekday: schedule.frequency === 'weekly' ? (schedule.weekday ?? 1) : null,
            monthDay: schedule.frequency === 'monthly' ? (schedule.monthDay ?? 1) : null,
          },
          recipients: pending,
          message,
        },
      },
      {
        onSuccess: (r) => {
          onSaved?.(r);
          onClose();
        },
      },
    );
  };

  return (
    <Modal onClose={onClose} label={report ? 'Edit report' : 'New report'} width={640} height="auto">
      <div className="small-modal rp-editor">
        <h3>
          {report ? 'Edit report' : 'Schedule a report'} <span className="muted-text">· {title}</span>
        </h3>
        <label className="rp-field">
          <span>Report name</span>
          <input autoFocus placeholder={`${title} report`} value={name} onChange={(e) => setName(e.target.value)} />
        </label>
        {vars.length > 0 && (
          <div className="rp-field">
            <span>Template variables</span>
            <div className="rp-vars">
              {vars.map((v) => (
                <label key={v.name} className="rp-var">
                  <span className="muted-text">${v.name}</span>
                  <input value={values[v.name] ?? v.default} onChange={(e) => setValues({ ...values, [v.name]: e.target.value })} placeholder="*" />
                </label>
              ))}
            </div>
          </div>
        )}
        <div className="rp-grid">
          <label className="rp-field">
            <span>Time frame</span>
            <select value={timeFrame} onChange={(e) => setTimeFrame(e.target.value)}>
              {(options.data?.timeFrames ?? []).map((t) => (
                <option key={t.key} value={t.key}>
                  {t.label}
                </option>
              ))}
            </select>
          </label>
          <label className="rp-field">
            <span>Frequency</span>
            <select value={schedule.frequency} onChange={(e) => set({ frequency: e.target.value as ReportFrequency })}>
              <option value="daily">Daily</option>
              <option value="weekly">Weekly</option>
              <option value="monthly">Monthly</option>
            </select>
          </label>
          {schedule.frequency === 'weekly' && (
            <label className="rp-field">
              <span>On</span>
              <select value={schedule.weekday ?? 1} onChange={(e) => set({ weekday: Number(e.target.value) })}>
                {WEEKDAYS.map((d, i) => (
                  <option key={d} value={i}>
                    {d}
                  </option>
                ))}
              </select>
            </label>
          )}
          {schedule.frequency === 'monthly' && (
            <label className="rp-field">
              <span>Day of month</span>
              <select value={schedule.monthDay ?? 1} onChange={(e) => set({ monthDay: Number(e.target.value) })}>
                {Array.from({ length: 28 }, (_, i) => (
                  <option key={i} value={i + 1}>
                    {i + 1}
                  </option>
                ))}
              </select>
            </label>
          )}
          <label className="rp-field">
            <span>At</span>
            <input type="time" value={schedule.time} onChange={(e) => set({ time: e.target.value })} />
          </label>
          <label className="rp-field">
            <span>Time zone</span>
            <input list="rp-zones" value={schedule.timezone} onChange={(e) => set({ timezone: e.target.value })} />
            <datalist id="rp-zones">
              {ZONES.map((z) => (
                <option key={z} value={z} />
              ))}
            </datalist>
          </label>
        </div>
        <p className="muted-text rp-when">{describeSchedule(schedule)}</p>
        <div className="rp-field">
          <span>Recipients</span>
          <div className="rp-recipients">
            {recipients.map((r) => (
              <span key={r} className="tag-chip rp-chip">
                {r.includes('@') ? r : `@${r}`}
                <button className="icon-plain" title="Remove" onClick={() => setRecipients((rs) => rs.filter((x) => x !== r))}>
                  <X size={12} />
                </button>
              </span>
            ))}
            <input
              list="rp-channels"
              placeholder={recipients.length ? 'Add more…' : 'name@example.com or @slack-channel'}
              value={draft}
              onChange={(e) => setDraft(e.target.value)}
              onKeyDown={(e) => {
                if (e.key === 'Enter' || e.key === ',') {
                  e.preventDefault();
                  addRecipient(draft);
                }
              }}
              onBlur={() => draft.trim() && addRecipient(draft)}
            />
            <datalist id="rp-channels">
              {channels.map((c) => (
                <option key={c} value={`@${c}`} />
              ))}
            </datalist>
          </div>
          <span className="muted-text rp-hint">Emails of members of this organization, or notification channels (Monitors → Settings → Notification Channels).</span>
          {emailOff && (
            <p className="rp-note">
              <Info size={14} /> Email isn't configured on this server (OC_SMTP_URL), so only channels receive reports for now.
            </p>
          )}
        </div>
        <label className="rp-field">
          <span>Message (optional)</span>
          <textarea rows={3} value={message} onChange={(e) => setMessage(e.target.value)} placeholder="Add context for the recipients" />
        </label>
        {save.error && <p className="form-error">{save.error.message}</p>}
        <div className="small-modal-actions">
          <button className="btn" onClick={onClose}>
            Cancel
          </button>
          <button className="btn btn-primary" disabled={save.isPending || !dash.data} onClick={submit}>
            {save.isPending ? 'Saving…' : report ? 'Save report' : 'Create report'}
          </button>
        </div>
      </div>
    </Modal>
  );
}
