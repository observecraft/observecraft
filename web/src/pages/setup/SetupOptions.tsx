import { useState } from 'react';
import { Link } from 'react-router-dom';
import { X } from 'lucide-react';
import { Modal } from '../../components/ui/Modal';
import { CopyButton } from '../settings/SecuritySettings';
import { fill, type SetupStep } from './sources';
import './setup.css';

/** One way to send a product's data (the Datadog Agent check, an OpenTelemetry receiver, …). */
export interface SetupOption {
  id: string;
  name: string;
  summary: string;
  steps: SetupStep[];
}

const intakeUrl = () => `${window.location.protocol}//${window.location.hostname}:4318`;

/** The steps of one option, with this install's intake filled in (the Add a Log Source guide's layout). */
export function SetupStepList({ option }: { option: SetupOption }) {
  const values = { intake: intakeUrl(), apiKey: '<API_KEY>' };
  return (
    <>
      <p className="muted">{option.summary}</p>
      <p className="setup-key-note">
        Replace <code>&lt;API_KEY&gt;</code> with a key from <Link to="/organization-settings/api-keys">API Keys</Link>. The intake for this install is <code>{values.intake}</code>.
      </p>
      <ol className="setup-steps">
        {option.steps.map((step) => (
          <li key={step.title}>
            <h3>{step.title}</h3>
            {step.text && <p>{step.text}</p>}
            {step.code && (
              <div className="key-snippet">
                <div className="key-snippet-head">
                  <span />
                  <CopyButton text={fill(step.code, values)} />
                </div>
                <pre>{fill(step.code, values)}</pre>
              </div>
            )}
          </li>
        ))}
      </ol>
    </>
  );
}

/** A choice between setup options (tabs) and the chosen option's steps. */
export function SetupOptions({ options, initial }: { options: SetupOption[]; initial?: string }) {
  const [id, setId] = useState(initial ?? options[0]?.id);
  const option = options.find((o) => o.id === id) ?? options[0];
  if (!option) return null;
  return (
    <div className="setup-options">
      <div className="btn-group">
        {options.map((o) => (
          <button key={o.id} className={`btn${o.id === option.id ? ' btn-active' : ''}`} onClick={() => setId(o.id)}>
            {o.name}
          </button>
        ))}
      </div>
      <SetupStepList option={option} />
    </div>
  );
}

/** The setup options of a product in a dialog (a product intro's "Get Started"). */
export function SetupModal({ title, options, initial, onClose }: { title: string; options: SetupOption[]; initial?: string; onClose: () => void }) {
  return (
    <Modal label={title} onClose={onClose} width={880} height="auto">
      <div className="setup-modal">
        <header>
          <h2>{title}</h2>
          <button className="icon-btn" onClick={onClose} title="Close">
            <X size={16} />
          </button>
        </header>
        <SetupOptions options={options} initial={initial} />
      </div>
    </Modal>
  );
}
