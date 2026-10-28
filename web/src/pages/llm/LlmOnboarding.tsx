import { useState } from 'react';
import { Link, useNavigate } from 'react-router-dom';
import { useQueryClient } from '@tanstack/react-query';
import { Bot, Wrench } from 'lucide-react';
import { CopyButton } from '../settings/SecuritySettings';
import { fill } from '../setup/sources';

const intakeUrl = () => `${window.location.protocol}//${window.location.hostname}:4318`;

type Method = 'agent' | 'manual';
type Sdk = 'otel' | 'openllmetry' | 'ddtrace';

const SDKS: { id: Sdk; label: string; note: string; code: string }[] = [
  {
    id: 'otel',
    label: 'OpenTelemetry (Python)',
    note: 'The OpenTelemetry GenAI instrumentations record model, tokens and, when enabled, the messages of each call. Other languages work the same way with their GenAI instrumentation.',
    code: [
      'pip install opentelemetry-distro opentelemetry-exporter-otlp opentelemetry-instrumentation-openai-v2',
      '',
      'export OTEL_SERVICE_NAME={{app}}',
      'export OTEL_EXPORTER_OTLP_ENDPOINT={{intake}}',
      'export OTEL_EXPORTER_OTLP_PROTOCOL=http/protobuf',
      'export OTEL_EXPORTER_OTLP_HEADERS="Authorization=Bearer%20{{apiKey}}"',
      '# Store prompts and answers too (leave out if they may hold sensitive data):',
      'export OTEL_INSTRUMENTATION_GENAI_CAPTURE_MESSAGE_CONTENT=true',
      '',
      'opentelemetry-instrument python app.py',
    ].join('\n'),
  },
  {
    id: 'openllmetry',
    label: 'OpenLLMetry',
    note: 'OpenLLMetry instruments the common LLM, vector database and framework libraries and sends OpenTelemetry spans.',
    code: [
      'pip install traceloop-sdk',
      '',
      'export TRACELOOP_BASE_URL={{intake}}',
      'export TRACELOOP_HEADERS="Authorization=Bearer%20{{apiKey}}"',
      '',
      '# At startup, before the LLM clients are created:',
      'from traceloop.sdk import Traceloop',
      'Traceloop.init(app_name="{{app}}")',
    ].join('\n'),
  },
  {
    id: 'ddtrace',
    label: 'dd-trace',
    note: 'With a Datadog Agent pointed at ObserveCraft, the APM spans of dd-trace arrive here. ObserveCraft reads the LLM Observability tags those spans carry; it does not accept the separate LLM Observability intake, so messages may be missing. Prefer OpenTelemetry for full content.',
    code: [
      '# Datadog Agent: send APM to ObserveCraft',
      'DD_API_KEY={{apiKey}}',
      'DD_APM_DD_URL={{intake}}',
      '',
      '# Application',
      'pip install ddtrace',
      'export DD_LLMOBS_ENABLED=1',
      'export DD_LLMOBS_ML_APP={{app}}',
      'export DD_SERVICE={{app}}',
      'ddtrace-run python app.py',
    ].join('\n'),
  },
];

const AGENT_PROMPT = [
  'Add LLM observability to this application with OpenTelemetry.',
  '',
  '- Use the OpenTelemetry GenAI semantic conventions (gen_ai.* attributes). Prefer the official OpenTelemetry instrumentation packages for the LLM client libraries this project already uses; add manual spans only where no instrumentation exists.',
  '- Export traces over OTLP/HTTP to {{intake}} (the SDK appends /v1/traces) with the header "Authorization: Bearer <key>". Read the key from an environment variable named OBSERVECRAFT_API_KEY; never write it into the code.',
  '- Set the resource attribute service.name to "{{app}}".',
  '- Give each user request one parent span, so its model calls, tool calls and retrieval steps share a trace. Name agent, tool and retrieval spans after what they do.',
  '- Record token usage. Capture prompt and completion content only behind a setting that is off by default, and tell me where to turn it on.',
  '- Do not change the behaviour of the application. List the files you changed and the environment variables to set.',
].join('\n');

/** Empty state and setup guide: how to send LLM spans to ObserveCraft. */
export function LlmOnboarding() {
  const navigate = useNavigate();
  const client = useQueryClient();
  const [method, setMethod] = useState<Method>('manual');
  const [sdk, setSdk] = useState<Sdk>('otel');
  const [app, setApp] = useState('support-assistant');
  const values = { intake: intakeUrl(), apiKey: '<API_KEY>' };
  const withApp = (template: string) => fill(template, values).replaceAll('{{app}}', app.trim() || 'my-llm-app');
  const chosen = SDKS.find((s) => s.id === sdk)!;
  return (
    <div className="llm-onboarding">
      <h1>Submit your first trace</h1>
      <p className="muted">
        LLM Observability reads the LLM spans in your traces: model calls, agents, tools and retrieval steps sent with the OpenTelemetry GenAI conventions, OpenLLMetry or dd-trace. Nothing else to
        deploy: they arrive through the same intake as the rest of your telemetry.
      </p>
      <div className="llm-divider">
        <span>Choose setup method</span>
      </div>
      <div className="llm-methods">
        <button className={`llm-method${method === 'agent' ? ' selected' : ''}`} onClick={() => setMethod('agent')}>
          <Bot size={22} />
          <b>Set up with your coding agent</b>
          <span className="muted">Copy a prompt that asks your AI coding assistant to add the instrumentation.</span>
        </button>
        <button className={`llm-method${method === 'manual' ? ' selected' : ''}`} onClick={() => setMethod('manual')}>
          <Wrench size={22} />
          <b>Set up manually</b>
          <span className="muted">Install an SDK and set a few environment variables.</span>
        </button>
      </div>
      <ol className="llm-steps">
        <li className="llm-step-plain">
          <label className="llm-field">
            <span>Application name</span>
            <input className="input" value={app} onChange={(e) => setApp(e.target.value)} placeholder="my-llm-app" />
          </label>
          <span className="muted">Shown as the ML application; the service name of the spans.</span>
        </li>
        <li>
          <h3>Instrument your application</h3>
          {method === 'agent' ? (
            <>
              <p className="muted">Paste this into your coding agent, in the repository of the application.</p>
              <Snippet text={withApp(AGENT_PROMPT)} />
            </>
          ) : (
            <>
              <div className="llm-seg">
                {SDKS.map((s) => (
                  <button key={s.id} className={sdk === s.id ? 'active' : ''} onClick={() => setSdk(s.id)}>
                    {s.label}
                  </button>
                ))}
              </div>
              <p className="muted">{chosen.note}</p>
              <Snippet text={withApp(chosen.code)} />
            </>
          )}
        </li>
        <li>
          <h3>Select an API key</h3>
          <p className="muted">
            Replace <code>&lt;API_KEY&gt;</code> with a key from <Link to="/organization-settings/api-keys">Organization Settings → API Keys</Link>. The intake for this install is{' '}
            <code>{values.intake}</code>.
          </p>
        </li>
        <li>
          <h3>Continue</h3>
          <p className="muted">Run the application and send it a request. Traces show up within a few seconds.</p>
          <button
            className="btn btn-primary"
            onClick={() => {
              void client.invalidateQueries({ queryKey: ['llm'] });
              navigate(`/llm/traces?app=${encodeURIComponent(app.trim())}`);
            }}
          >
            Continue to Traces
          </button>
        </li>
      </ol>
    </div>
  );
}

function Snippet({ text }: { text: string }) {
  return (
    <div className="key-snippet">
      <div className="key-snippet-head">
        <span />
        <CopyButton text={text} />
      </div>
      <pre>{text}</pre>
    </div>
  );
}
