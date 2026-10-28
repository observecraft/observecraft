import { useState } from 'react';
import { X } from 'lucide-react';
import {
  COMPONENT_STATUS_LABEL,
  NOTICE_STATUSES,
  NOTICE_STATUS_LABEL,
  type ComponentStatus,
  type NoticeImpact,
  type NoticeInput,
  type NoticeStatus,
  type NoticeUpdateInput,
  type StatusPageComponent,
  type StatusPageNotice,
} from '../../api/statusPages/types';
import { Modal } from '../../components/ui/Modal';

const IMPACTS: ComponentStatus[] = ['degraded_performance', 'partial_outage', 'major_outage'];

/**
 * Opens a notice (title, first message, impacts, optional incident) or, with `notice`, posts an
 * update to it: a new status and message, and the impacts from then on.
 */
export function NoticeModal({
  components,
  notice,
  pending,
  error,
  onCreate,
  onUpdate,
  onClose,
}: {
  components: StatusPageComponent[];
  notice?: StatusPageNotice;
  pending: boolean;
  error: Error | null;
  onCreate: (i: NoticeInput) => void;
  onUpdate: (i: NoticeUpdateInput) => void;
  onClose: () => void;
}) {
  const [title, setTitle] = useState('');
  const [status, setStatus] = useState<NoticeStatus>(notice ? (notice.status === 'investigating' ? 'identified' : notice.status) : 'investigating');
  const [message, setMessage] = useState('');
  const [incident, setIncident] = useState('');
  const [impacts, setImpacts] = useState<Record<string, ComponentStatus | ''>>(() => Object.fromEntries((notice?.impacts ?? []).map((i) => [i.componentId, i.impact])));
  const incidentId = incident.trim() ? Number(incident.trim().replace(/^IR-/i, '')) : null;
  const incidentError = incidentId !== null && !(Number.isInteger(incidentId) && incidentId > 0) ? 'Enter an incident number such as IR-12' : null;
  const ready = Boolean((notice || title.trim()) && message.trim() && !incidentError);
  const chosen: NoticeImpact[] = components.flatMap((c) => (impacts[c.id] ? [{ componentId: c.id, impact: impacts[c.id] as ComponentStatus }] : []));
  const submit = () => (notice ? onUpdate({ status, message, impacts: chosen }) : onCreate({ title: title.trim(), status, message, impacts: chosen, incidentId: incidentError ? null : incidentId }));

  return (
    <Modal onClose={onClose} label={notice ? 'Post an update' : 'New notice'} width={720} height="auto">
      <form
        className="small-modal ar-form"
        onSubmit={(e) => {
          e.preventDefault();
          if (ready) submit();
        }}
      >
        <div className="modal-title-row">
          <h3>{notice ? `Update: ${notice.title}` : 'New Notice'}</h3>
          <button type="button" className="icon-plain" onClick={onClose} title="Close">
            <X size={18} />
          </button>
        </div>
        <div className="stp-modal-scroll small-modal">
          {!notice && (
            <label className="form-label">
              Title
              <input value={title} onChange={(e) => setTitle(e.target.value)} placeholder="Checkout errors for some customers" maxLength={200} autoFocus />
            </label>
          )}
          <div className="stp-row">
            <label className="form-label">
              Status
              <select value={status} onChange={(e) => setStatus(e.target.value as NoticeStatus)}>
                {NOTICE_STATUSES.map((s) => (
                  <option key={s} value={s}>
                    {NOTICE_STATUS_LABEL[s]}
                  </option>
                ))}
              </select>
            </label>
            {!notice && (
              <label className="form-label">
                Incident (optional)
                <input value={incident} onChange={(e) => setIncident(e.target.value)} placeholder="IR-12" />
              </label>
            )}
          </div>
          {incidentError && <p className="form-error">{incidentError}</p>}
          <label className="form-label">
            Message — Markdown
            <textarea
              rows={5}
              value={message}
              onChange={(e) => setMessage(e.target.value)}
              placeholder="What users notice, what is being done and when the next update comes"
              maxLength={10_000}
              autoFocus={Boolean(notice)}
            />
          </label>
          {components.length > 0 && (
            <div className="form-label">
              Affected components{status === 'resolved' && ' — a resolved notice no longer affects them'}
              {components.map((c) => (
                <div key={c.id} className="stp-impact-row">
                  <span>
                    {c.group && <span className="muted">{c.group} / </span>}
                    {c.name}
                  </span>
                  <select aria-label={`Impact on ${c.name}`} value={impacts[c.id] ?? ''} onChange={(e) => setImpacts((m) => ({ ...m, [c.id]: e.target.value as ComponentStatus | '' }))}>
                    <option value="">Not affected</option>
                    {IMPACTS.map((s) => (
                      <option key={s} value={s}>
                        {COMPONENT_STATUS_LABEL[s]}
                      </option>
                    ))}
                  </select>
                </div>
              ))}
            </div>
          )}
        </div>
        {error && <p className="form-error">{error.message}</p>}
        <div className="small-modal-actions">
          <button type="button" className="btn" onClick={onClose}>
            Cancel
          </button>
          <button className="btn btn-primary" disabled={!ready || pending}>
            {pending ? 'Posting…' : notice ? 'Post Update' : 'Post Notice'}
          </button>
        </div>
      </form>
    </Modal>
  );
}
