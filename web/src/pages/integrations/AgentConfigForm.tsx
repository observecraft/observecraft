// The Datadog Agent check's configuration form, generated from its integrations-core spec, and the
// conf.yaml it produces. Values stay in the browser: the page doesn't deploy configuration.
import { useMemo, useState } from 'react';
import { formValue, instanceOptions, renderAgentConfig, type SpecOption } from '../../api/integrations/agentConfig';
import { CodeBlock } from '../../components/code/CodeBlock';
import { ErrorState, Loading } from '../../components/ui/AsyncState';
import { useAgentSpec } from './queries';

/** First sentence of an option's description (the full text is the field's tooltip). */
const summary = (text = '') => text.split(/\n\n|(?<=\.)\s/)[0];

function placeholder(o: SpecOption): string {
  const v = formValue(o.value);
  const sample = o.value?.example ?? o.value?.display_default ?? v?.example ?? v?.display_default;
  if (sample === undefined || sample === null) return v?.type === 'array' ? 'Comma-separated values' : '';
  return Array.isArray(sample) ? sample.join(', ') : typeof sample === 'object' ? '' : String(sample);
}

function Field({ option, value, onChange }: { option: SpecOption; value: string; onChange: (value: string) => void }) {
  const type = formValue(option.value)?.type;
  return (
    <label title={option.description}>
      <span>
        <span className="mono">{option.name}</span>
        {option.required && <span className="im-required"> *</span>}
      </span>
      {type === 'boolean' ? (
        <select value={value} onChange={(e) => onChange(e.target.value)}>
          <option value="">Default{placeholder(option) ? ` (${placeholder(option)})` : ''}</option>
          <option value="true">true</option>
          <option value="false">false</option>
        </select>
      ) : (
        <input
          type={option.secret ? 'password' : type === 'integer' || type === 'number' ? 'number' : 'text'}
          placeholder={placeholder(option)}
          value={value}
          autoComplete="off"
          onChange={(e) => onChange(e.target.value)}
        />
      )}
      {option.description && <small className="muted-text">{summary(option.description)}</small>}
    </label>
  );
}

export function AgentConfigForm({ check }: { check: string }) {
  const spec = useAgentSpec(check);
  const [values, setValues] = useState<Record<string, string>>({});
  const [showOptional, setShowOptional] = useState(false);
  const options = useMemo(() => (spec.data ? instanceOptions(spec.data) : []), [spec.data]);

  if (spec.isPending) return <Loading label="Loading configuration options…" />;
  if (spec.isError) return <ErrorState error={spec.error} onRetry={() => spec.refetch()} />;
  if (!spec.data) return null;

  const required = options.filter((o) => o.required);
  const optional = options.filter((o) => !o.required);
  const field = (o: SpecOption) => <Field key={o.name} option={o} value={values[o.name] ?? ''} onChange={(v) => setValues((prev) => ({ ...prev, [o.name]: v }))} />;

  return (
    <>
      <h3>Agent configuration</h3>
      <div className="im-settings im-agent-settings">
        {required.map(field)}
        {optional.length > 0 && (
          <button type="button" className="link-button" onClick={() => setShowOptional((s) => !s)}>
            {showOptional ? 'Hide' : 'Show'} {optional.length} optional settings
          </button>
        )}
        {showOptional && optional.map(field)}
      </div>
      <p className="muted-text">Keep passwords and tokens out of the file with the Agent's secrets management: write ENC[&lt;handle&gt;] instead of the value.</p>
      <CodeBlock code={renderAgentConfig(spec.data, values)} filename={`conf.d/${check}.d/conf.yaml`} badge={<span className="cfg-target cfg-datadog-agent">Datadog Agent</span>} />
    </>
  );
}
