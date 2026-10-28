import { useState } from 'react';
import { Plus, Trash2, X } from 'lucide-react';
import type { ScorecardService, ServiceContact } from '../../api/scorecards/client';
import { Modal } from '../../components/ui/Modal';
import { useTeams } from '../settings/queries';
import { useUpdateServiceMetadata } from './queries';

const CONTACT_TYPES: { id: string; label: string; placeholder: string }[] = [
  { id: 'email', label: 'Email', placeholder: 'checkout-oncall@example.com' },
  { id: 'slack', label: 'Chat channel', placeholder: '#checkout' },
  { id: 'link', label: 'Link', placeholder: 'https://wiki.example.com/checkout' },
];

/** Edits a service's team, contacts, code repository and docs, which the ownership rules read. */
export function MetadataModal({ service, onClose }: { service: ScorecardService; onClose: () => void }) {
  const [team, setTeam] = useState(service.team);
  const [contacts, setContacts] = useState<ServiceContact[]>(service.contacts.length ? service.contacts : []);
  const [repoUrl, setRepoUrl] = useState(service.repoUrl);
  const [docsUrl, setDocsUrl] = useState(service.docsUrl);
  const teams = useTeams({ limit: 200, offset: 0, sort: 'name' });
  const save = useUpdateServiceMetadata();
  const setContact = (i: number, patch: Partial<ServiceContact>) => setContacts((cs) => cs.map((c, j) => (j === i ? { ...c, ...patch } : c)));

  return (
    <Modal label={`Metadata of ${service.service}`} onClose={onClose} width={620} height="auto">
      <div className="sc-modal">
        <header className="sc-modal-head">
          <h2>Service metadata · {service.service}</h2>
          <button className="sc-icon-btn" aria-label="Close" onClick={onClose}>
            <X size={18} />
          </button>
        </header>
        <div className="sc-form">
          <label className="sc-field">
            <span>Owning team</span>
            <input className="sc-input" list="sc-teams" value={team} onChange={(e) => setTeam(e.target.value)} placeholder="Team handle" />
            <datalist id="sc-teams">
              {teams.data?.teams.map((t) => (
                <option key={t.handle} value={t.handle}>
                  {t.name}
                </option>
              ))}
            </datalist>
          </label>
          <div className="sc-field">
            <span>Contacts</span>
            {contacts.map((c, i) => (
              <div key={i} className="sc-contact-row">
                <select className="sc-input" value={c.type} onChange={(e) => setContact(i, { type: e.target.value })} aria-label="Contact type">
                  {CONTACT_TYPES.map((t) => (
                    <option key={t.id} value={t.id}>
                      {t.label}
                    </option>
                  ))}
                </select>
                <input className="sc-input" value={c.name ?? ''} onChange={(e) => setContact(i, { name: e.target.value })} placeholder="Name (optional)" aria-label="Contact name" />
                <input
                  className="sc-input grow"
                  value={c.value}
                  onChange={(e) => setContact(i, { value: e.target.value })}
                  placeholder={CONTACT_TYPES.find((t) => t.id === c.type)?.placeholder}
                  aria-label="Contact"
                />
                <button className="sc-icon-btn" aria-label="Remove contact" onClick={() => setContacts((cs) => cs.filter((_, j) => j !== i))}>
                  <Trash2 size={14} />
                </button>
              </div>
            ))}
            <button className="btn btn-sm sc-add" onClick={() => setContacts((cs) => [...cs, { type: 'email', name: '', value: '' }])}>
              <Plus size={14} /> Add contact
            </button>
          </div>
          <label className="sc-field">
            <span>Code repository</span>
            <input className="sc-input" value={repoUrl} onChange={(e) => setRepoUrl(e.target.value)} placeholder="https://git.example.com/example-org/order-service" />
          </label>
          <label className="sc-field">
            <span>Documentation or runbook</span>
            <input className="sc-input" value={docsUrl} onChange={(e) => setDocsUrl(e.target.value)} placeholder="https://docs.example.com/services/order-service" />
          </label>
          {save.isError && <div className="form-error">{(save.error as Error).message}</div>}
          <div className="sc-modal-actions">
            <button className="btn" onClick={onClose}>
              Cancel
            </button>
            <button
              className="btn btn-primary"
              disabled={save.isPending}
              onClick={() => save.mutate({ service: service.service, input: { team, contacts: contacts.filter((c) => c.value.trim()), repoUrl, docsUrl } }, { onSuccess: onClose })}
            >
              {save.isPending ? 'Saving…' : 'Save'}
            </button>
          </div>
        </div>
      </div>
    </Modal>
  );
}
