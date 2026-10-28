import { useState } from 'react';
import { X } from 'lucide-react';
import type { CorrectionCategory, SloCorrection, SloCorrectionInput, SloItem } from '../../api/slos/client';
import { Modal } from '../../components/ui/Modal';
import { CATEGORY_LABEL } from './common';
import { useSaveCorrection } from './queries';

/** `<input type="datetime-local">` value for an epoch ms, in local time. */
function localInput(ms: number): string {
  const d = new Date(ms - new Date(ms).getTimezoneOffset() * 60_000);
  return d.toISOString().slice(0, 16);
}

const HOUR = 3_600_000;

/** Create or edit a correction: a time range excluded from one SLO, or from every SLO. */
export function CorrectionModal({ onClose, correction, sloId, slos }: { onClose: () => void; correction?: SloCorrection; sloId?: string; slos: Pick<SloItem, 'id' | 'name'>[] }) {
  const save = useSaveCorrection(correction?.id);
  const now = Math.floor(Date.now() / (5 * 60_000)) * 5 * 60_000;
  const [slo, setSlo] = useState(correction ? (correction.sloId ?? '') : (sloId ?? ''));
  const [category, setCategory] = useState<CorrectionCategory>(correction?.category ?? 'scheduled_maintenance');
  const [description, setDescription] = useState(correction?.description ?? '');
  const [start, setStart] = useState(localInput(correction?.start ?? now - HOUR));
  const [end, setEnd] = useState(localInput(correction?.end ?? now));
  const [error, setError] = useState<string | null>(null);

  const submit = () => {
    const input: SloCorrectionInput = { sloId: slo || null, category, description, start: new Date(start).getTime(), end: new Date(end).getTime() };
    save.mutate(input, { onSuccess: onClose, onError: (err) => setError((err as Error).message) });
  };

  return (
    <Modal onClose={onClose} label={correction ? 'Edit correction' : 'New correction'} width={560} height="auto">
      <form
        className="small-modal mon-dt-form"
        onSubmit={(e) => {
          e.preventDefault();
          submit();
        }}
      >
        <div className="modal-title-row">
          <h3>{correction ? 'Edit Correction' : 'New Correction'}</h3>
          <button type="button" className="icon-plain" onClick={onClose} title="Close">
            <X size={18} />
          </button>
        </div>
        <label className="form-label">
          SLO
          <select value={slo} onChange={(e) => setSlo(e.target.value)} aria-label="SLO">
            <option value="">Every SLO</option>
            {slos.map((s) => (
              <option key={s.id} value={s.id}>
                {s.name}
              </option>
            ))}
          </select>
        </label>
        <label className="form-label">
          Reason
          <select value={category} onChange={(e) => setCategory(e.target.value as CorrectionCategory)} aria-label="Reason">
            {(Object.keys(CATEGORY_LABEL) as CorrectionCategory[]).map((c) => (
              <option key={c} value={c}>
                {CATEGORY_LABEL[c]}
              </option>
            ))}
          </select>
        </label>
        <div className="slo-form-pair">
          <label className="form-label">
            Start
            <input type="datetime-local" value={start} onChange={(e) => setStart(e.target.value)} required />
          </label>
          <label className="form-label">
            End
            <input type="datetime-local" value={end} onChange={(e) => setEnd(e.target.value)} required />
          </label>
        </div>
        <label className="form-label">
          Description
          <textarea rows={2} placeholder="e.g. Planned database upgrade" value={description} onChange={(e) => setDescription(e.target.value)} />
        </label>
        <p className="muted">Events and time within the range don't count towards the SLI or the error budget.</p>
        {error && <p className="form-error">{error}</p>}
        <div className="small-modal-actions">
          <button type="button" className="btn" onClick={onClose}>
            Cancel
          </button>
          <button className="btn btn-primary" disabled={save.isPending}>
            {correction ? 'Save' : 'Create'}
          </button>
        </div>
      </form>
    </Modal>
  );
}
