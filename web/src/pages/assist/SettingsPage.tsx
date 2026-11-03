import { useEffect, useState } from 'react';
import { Link } from 'react-router-dom';
import { ArrowLeft, Search } from 'lucide-react';
import { ErrorState, Loading } from '../../components/ui/AsyncState';
import { NotConfigured } from './ChatPage';
import { useAssistSettings, useAssistStatus, useUpdateAssistSettings } from './queries';
import './assist.css';

/** Assist → Settings: the user's own context for the model and the skills (groups of tools) it may use. */
export function AssistSettingsPage() {
  const status = useAssistStatus();
  const settings = useAssistSettings();
  const save = useUpdateAssistSettings();
  const [context, setContext] = useState('');
  const [query, setQuery] = useState('');
  useEffect(() => {
    if (settings.data) setContext(settings.data.context);
  }, [settings.data]);
  if (settings.isPending || status.isPending) return <Loading label="Loading Assist settings…" />;
  if (settings.isError) return <ErrorState error={settings.error} onRetry={() => settings.refetch()} />;
  if (status.isError) return <ErrorState error={status.error} onRetry={() => status.refetch()} />;
  const disabled = settings.data.disabledSkills;
  const toggle = (name: string) => save.mutate({ context: settings.data.context, disabledSkills: disabled.includes(name) ? disabled.filter((d) => d !== name) : [...disabled, name] });
  const text = query.trim().toLowerCase();
  const skills = status.data.skills.filter((s) => !text || s.name.includes(text) || s.description.toLowerCase().includes(text) || s.tools.some((t) => t.includes(text)));
  return (
    <div className="ai-settings">
      <Link className="btn btn-sm ai-back" to="/assist">
        <ArrowLeft size={14} /> Back
      </Link>
      {!status.data.configured && <NotConfigured error={status.data.error} />}
      <section>
        <h2>User Context</h2>
        <p className="muted">Assist keeps this in mind in every conversation: your role, the services you own, how you like answers.</p>
        <textarea className="ai-context" rows={4} maxLength={4000} value={context} placeholder="Interests, preferences, or instructions to keep in mind" onChange={(e) => setContext(e.target.value)} />
        <div className="ai-row">
          <button className="btn btn-primary btn-sm" disabled={save.isPending || context === settings.data.context} onClick={() => save.mutate({ context, disabledSkills: disabled })}>
            Save
          </button>
          {save.isSuccess && !save.isPending && <span className="ai-ok">Saved</span>}
          {save.error && <span className="ai-bad">{save.error.message}</span>}
        </div>
      </section>
      <section>
        <h2>
          Skills <span className="ai-count">{status.data.skills.length}</span>
        </h2>
        <p className="muted">What Assist can use. Skills that change something ask for your approval before each action.</p>
        <div className="ai-filter">
          <Search size={15} />
          <input placeholder="Search skills" value={query} onChange={(e) => setQuery(e.target.value)} />
        </div>
        <table className="ai-skills">
          <thead>
            <tr>
              <th />
              <th>NAME</th>
              <th>DESCRIPTION</th>
              <th>TOOLS</th>
            </tr>
          </thead>
          <tbody>
            {skills.map((s) => (
              <tr key={s.name}>
                <td>
                  <button
                    type="button"
                    role="switch"
                    aria-label={`Use ${s.name}`}
                    aria-checked={!disabled.includes(s.name)}
                    className={`switch${disabled.includes(s.name) ? '' : ' on'}`}
                    disabled={save.isPending}
                    onClick={() => toggle(s.name)}
                  />
                </td>
                <td className="ai-skill-name">
                  {s.name}
                  {s.writes && <span className="ai-tool-tag">asks first</span>}
                </td>
                <td>{s.description}</td>
                <td className="muted ai-skill-tools">{s.tools.join(', ')}</td>
              </tr>
            ))}
            {!skills.length && (
              <tr>
                <td colSpan={4} className="muted">
                  No skills match.
                </td>
              </tr>
            )}
          </tbody>
        </table>
      </section>
      <section>
        <h2>Model</h2>
        <p className="muted">
          {status.data.configured ? (
            <>
              <b>{status.data.model}</b> through {status.data.provider === 'anthropic' ? 'Anthropic' : 'an OpenAI-compatible server'},{' '}
              {status.data.source === 'org' ? "this organization's provider" : "the install's default"}. Admins change it in{' '}
              <Link to="/organization-settings/assist">Organization Settings → Assist AI</Link>.
            </>
          ) : (
            'Not set up.'
          )}
        </p>
      </section>
    </div>
  );
}
