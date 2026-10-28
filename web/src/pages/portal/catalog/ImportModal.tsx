import { useState } from 'react';
import { useNavigate } from 'react-router-dom';
import { FileUp, X } from 'lucide-react';
import { API_MODE } from '../../../api/http';
import type { PortalImportResult } from '../../../api/portalCatalog/types';
import { Modal } from '../../../components/ui/Modal';
import { useImportDefinitions } from './queries';

const EXAMPLE_YAML = `apiVersion: v3
kind: service
metadata:
  name: order-service
  displayName: Order Service
  owner: checkout
  description: Takes and tracks orders.
  tags:
    - tier:1
  links:
    - name: Source
      type: repo
      url: https://git.example.com/example-org/order-service
    - name: Runbook
      type: runbook
      url: https://docs.example.com/runbooks/order-service
  contacts:
    - name: On-call
      type: email
      contact: checkout-oncall@example.com
spec:
  lifecycle: production
  tier: "1"
  type: web
  languages:
    - go
  dependsOn:
    - service:billing
    - datastore:orders-db
  componentOf:
    - system:checkout
`;

const EXAMPLE_JSON = JSON.stringify(
  [
    {
      apiVersion: 'v3',
      kind: 'service',
      metadata: { name: 'order-service', owner: 'checkout', description: 'Takes and tracks orders.', tags: ['tier:1'] },
      spec: { lifecycle: 'production', tier: '1', dependsOn: ['service:billing', 'datastore:orders-db'], componentOf: ['system:checkout'] },
    },
  ],
  null,
  2,
);

/** Import entity definition documents (apiVersion v3): pasted or from files. */
export function ImportModal({ onClose }: { onClose: () => void }) {
  const [content, setContent] = useState('');
  const [result, setResult] = useState<PortalImportResult | null>(null);
  const imp = useImportDefinitions();
  const navigate = useNavigate();
  const readFiles = async (files: FileList | null) => {
    if (!files?.length) return;
    const texts = await Promise.all([...files].map((f) => f.text()));
    setContent(texts.map((t) => t.trim()).join('\n---\n'));
  };
  return (
    <Modal onClose={onClose} label="Import entity definitions" width={760} height="auto">
      <div className="pc-modal">
        <header className="pc-modal-head">
          <h2>Import entity definitions</h2>
          <button className="pc-icon-btn" aria-label="Close" onClick={onClose}>
            <X size={18} />
          </button>
        </header>
        {result ? (
          <div className="pc-modal-body">
            <p>
              {result.created} created, {result.updated} updated.
            </p>
            <ul className="pc-import-list">
              {result.entities.map((e) => (
                <li key={e.id}>
                  <button className="sc-link" onClick={() => navigate(`/portal/catalog?entity=${encodeURIComponent(`${e.kind}:${e.name}`)}`)}>
                    {e.kind}:{e.name}
                  </button>
                </li>
              ))}
            </ul>
            <div className="pc-modal-actions">
              <button className="btn btn-primary" onClick={onClose}>
                Done
              </button>
            </div>
          </div>
        ) : (
          <div className="pc-modal-body">
            <p className="sc-muted">
              One or more documents, YAML (separated by <code>---</code>) or JSON (an object or an array). Each one creates or replaces the definition of its kind and name; nothing is saved if one is
              invalid.
              {API_MODE !== 'live' && ' In this demo, documents are read as JSON.'}
            </p>
            <textarea
              className="pc-code-input"
              value={content}
              onChange={(e) => setContent(e.target.value)}
              spellCheck={false}
              placeholder={API_MODE === 'live' ? EXAMPLE_YAML : EXAMPLE_JSON}
              aria-label="Entity definitions"
            />
            {imp.isError && <div className="pc-error">{(imp.error as Error).message}</div>}
            <div className="pc-modal-actions">
              <label className="btn">
                <FileUp size={14} /> Upload files
                <input type="file" accept=".yaml,.yml,.json" multiple hidden onChange={(e) => readFiles(e.target.files)} />
              </label>
              <button className="btn" onClick={() => setContent(API_MODE === 'live' ? EXAMPLE_YAML : EXAMPLE_JSON)}>
                Use an example
              </button>
              <span className="pc-spacer" />
              <button className="btn" onClick={onClose}>
                Cancel
              </button>
              <button className="btn btn-primary" disabled={!content.trim() || imp.isPending} onClick={() => imp.mutate(content, { onSuccess: setResult })}>
                {imp.isPending ? 'Importing…' : 'Import'}
              </button>
            </div>
          </div>
        )}
      </div>
    </Modal>
  );
}
