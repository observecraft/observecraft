import { useState } from 'react';
import { X } from 'lucide-react';
import { CodeBlock } from '../../components/code/CodeBlock';
import { Modal } from '../../components/ui/Modal';

const LANGS = [
  { key: 'java', label: 'Java', run: 'java -javaagent:opentelemetry-javaagent.jar -jar app.jar' },
  { key: 'nodejs', label: 'Node.js', run: 'node --require @opentelemetry/auto-instrumentations-node/register app.js' },
  { key: 'python', label: 'Python', run: 'opentelemetry-instrument python app.py' },
  { key: 'dotnet', label: '.NET', run: '. $HOME/.otel-dotnet-auto/instrument.sh && dotnet app.dll' },
  { key: 'go', label: 'Go', run: '# Go has no agent: wrap handlers/clients with go.opentelemetry.io/contrib instrumentation\ngo run .' },
];

/** OpenTelemetry setup: every SDK reads the same env vars, so one snippet covers each language. */
export function InstrumentModal({ onClose }: { onClose: () => void }) {
  const [service, setService] = useState('my-service');
  const [env, setEnv] = useState('prod');
  const [lang, setLang] = useState(LANGS[0]);
  const safe = (s: string) => s.replace(/[^\w.-]/g, '') || 'my-service';
  const code = `# Send traces to the ObserveCraft collector (OTLP/HTTP)
export OTEL_EXPORTER_OTLP_ENDPOINT="http://<collector-host>:4318"
export OTEL_SERVICE_NAME="${safe(service)}"
export OTEL_RESOURCE_ATTRIBUTES="deployment.environment=${safe(env)},service.version=1.0.0"
export OTEL_TRACES_EXPORTER="otlp"

${lang.run}`;
  return (
    <Modal onClose={onClose} label="Instrument a service" width={760} height="auto">
      <div className="instrument-modal">
        <header>
          <h2>Instrument a Service</h2>
          <button className="icon-plain" onClick={onClose} title="Close (Esc)">
            <X size={20} />
          </button>
        </header>
        <p className="muted">ObserveCraft APM ingests OpenTelemetry traces. Point any OTel SDK or agent at the collector and the service appears here within a minute.</p>
        <div className="instrument-form">
          <label>
            Service name
            <input className="apm-input" value={service} onChange={(e) => setService(e.target.value)} />
          </label>
          <label>
            Environment
            <input className="apm-input" value={env} onChange={(e) => setEnv(e.target.value)} />
          </label>
        </div>
        <div className="apm-seg">
          {LANGS.map((l) => (
            <button key={l.key} className={l === lang ? 'active' : ''} onClick={() => setLang(l)}>
              {l.label}
            </button>
          ))}
        </div>
        <CodeBlock code={code} filename="shell" />
      </div>
    </Modal>
  );
}
