import { useEffect, useState } from 'react';
import { AlertTriangle, CheckCircle2, LoaderCircle, PlugZap } from 'lucide-react';
import type { AssistProviderUpdate, AssistProviderView } from '../../api/assist/client';
import { ErrorState, Loading } from '../../components/ui/AsyncState';
import { useAssistProvider, useDeleteAssistProvider, useSaveAssistProvider, useTestAssistProvider } from '../assist/queries';

type Mode = 'install' | 'org';

interface Form {
  mode: Mode;
  provider: 'anthropic' | 'openai';
  model: string;
  baseUrl: string;
  fallbacks: boolean;
  /** Empty keeps the stored key. */
  apiKey: string;
  removeKey: boolean;
}

function toForm(v: AssistProviderView): Form {
  const o = v.org;
  return {
    mode: o ? 'org' : 'install',
    provider: o?.provider === 'openai' ? 'openai' : 'anthropic',
    model: o?.model ?? '',
    baseUrl: o?.provider === 'openai' || (o?.baseUrl && o.baseUrl !== 'https://api.anthropic.com') ? (o?.baseUrl ?? '') : '',
    fallbacks: o?.fallbacks ?? true,
    apiKey: '',
    removeKey: false,
  };
}

function toUpdate(f: Form): AssistProviderUpdate {
  return {
    provider: f.provider,
    model: f.model.trim() || null,
    baseUrl: f.baseUrl.trim() || null,
    fallbacks: f.provider === 'anthropic' ? f.fallbacks : null,
    apiKey: f.removeKey ? '' : f.apiKey.trim() || null,
  };
}

const SOURCE: Record<string, string> = { org: "this organization's provider", install: "the install's default", none: 'nothing: Assist is off' };

/** Organization Settings → Assist AI: the model provider Assist uses for this organization. */
export function AssistProviderPage() {
  const q = useAssistProvider();
  const save = useSaveAssistProvider();
  const remove = useDeleteAssistProvider();
  const test = useTestAssistProvider();
  const [form, setForm] = useState<Form | null>(null);

  useEffect(() => {
    if (q.data && !form) setForm(toForm(q.data));
  }, [q.data, form]);

  if (q.isPending || !form) return q.isError ? <ErrorState error={q.error} onRetry={() => q.refetch()} /> : <Loading />;
  const v = q.data!;
  const set = (p: Partial<Form>) => {
    setForm({ ...form, ...p });
    test.reset();
  };
  const busy = save.isPending || remove.isPending;
  const error = (save.error ?? remove.error) as Error | null;
  const onSave = () => {
    if (form.mode === 'install') {
      if (v.org) remove.mutate(undefined, { onSuccess: () => setForm(null) });
      return;
    }
    save.mutate(toUpdate(form), { onSuccess: () => setForm(null) });
  };
  const inUse =
    v.source === 'org' && v.org
      ? `${v.org.model} (${v.org.provider === 'anthropic' ? 'Anthropic' : v.org.baseUrl})`
      : v.install
        ? `${v.install.model} (${v.install.provider === 'anthropic' ? 'Anthropic' : 'OpenAI-compatible'})`
        : null;
  const openai = form.provider === 'openai';
  const keyHint = v.org?.hasKey && !form.removeKey ? 'A key is stored. Leave empty to keep it.' : openai ? 'Optional for servers without authentication.' : 'Required.';

  return (
    <div className="settings-pad ap-page">
      <div className="settings-title-row">
        <h1 className="settings-title">Assist AI</h1>
        <button className="btn btn-primary" disabled={busy || (form.mode === 'install' && !v.org)} onClick={onSave}>
          {busy ? 'Saving…' : 'Save'}
        </button>
      </div>
      {error && (
        <p className="form-error lm-banner">
          <AlertTriangle size={15} /> {error.message}
        </p>
      )}
      {(save.isSuccess || remove.isSuccess) && !busy && (
        <p className="form-ok lm-banner">
          <CheckCircle2 size={15} /> Saved. New messages use it.
        </p>
      )}

      <section className="settings-card">
        <h2>In use</h2>
        <p>
          Assist uses {SOURCE[v.source] ?? v.source}
          {inUse && (
            <>
              : <b>{inUse}</b>
            </>
          )}
          .
        </p>
        <p className="muted">Questions, the data Assist reads to answer them and each user's context are sent to this provider.</p>
      </section>

      <section className="settings-card">
        <h2>Model provider</h2>
        <label className="lm-toggle">
          <input type="radio" name="ap-mode" checked={form.mode === 'install'} onChange={() => set({ mode: 'install' })} />
          <span>
            <b>The install's default</b>
            <span className="muted">{v.install ? `${v.install.model}, set by the operators of this install.` : 'None is set on this install: Assist stays off.'}</span>
          </span>
        </label>
        <label className="lm-toggle">
          <input type="radio" name="ap-mode" checked={form.mode === 'org'} onChange={() => set({ mode: 'org' })} />
          <span>
            <b>This organization's own</b>
            <span className="muted">Your key and model: Anthropic, or any OpenAI-compatible server (OpenAI, Azure OpenAI, vLLM, LiteLLM, Ollama).</span>
          </span>
        </label>

        {form.mode === 'org' && (
          <div className="ap-fields">
            <label className="form-label">
              Provider
              <select className="settings-input" value={form.provider} onChange={(e) => set({ provider: e.target.value as Form['provider'] })}>
                <option value="anthropic">Anthropic</option>
                <option value="openai">OpenAI-compatible</option>
              </select>
            </label>
            <label className="form-label">
              Model
              <input className="settings-input" value={form.model} placeholder={openai ? 'gpt-5, qwen3-coder, …' : 'claude-opus-5'} onChange={(e) => set({ model: e.target.value })} />
            </label>
            <label className="form-label">
              Base URL
              <input
                className="settings-input"
                value={form.baseUrl}
                placeholder={openai ? 'https://api.openai.com/v1' : 'https://api.anthropic.com'}
                onChange={(e) => set({ baseUrl: e.target.value })}
              />
              {!v.allowPrivate && <span className="muted ap-hint">https, on a public address.</span>}
            </label>
            <label className="form-label">
              API key
              <input
                className="settings-input"
                type="password"
                autoComplete="off"
                value={form.apiKey}
                disabled={form.removeKey}
                placeholder={v.org?.hasKey && !form.removeKey ? '••••••••' : ''}
                onChange={(e) => set({ apiKey: e.target.value })}
              />
              <span className="muted ap-hint">{keyHint}</span>
            </label>
            {openai && v.org?.hasKey && (
              <label className="lm-toggle">
                <input type="checkbox" className="checkbox" checked={form.removeKey} onChange={(e) => set({ removeKey: e.target.checked, apiKey: '' })} />
                <span>Remove the stored key</span>
              </label>
            )}
            {!openai && (
              <label className="lm-toggle">
                <input type="checkbox" className="checkbox" checked={form.fallbacks} onChange={(e) => set({ fallbacks: e.target.checked })} />
                <span>
                  <b>Fall back to another model when this one is overloaded</b>
                </span>
              </label>
            )}
            <div className="ap-test">
              <button className="btn" disabled={test.isPending} onClick={() => test.mutate(toUpdate(form))}>
                {test.isPending ? <LoaderCircle size={14} className="ai-spin" /> : <PlugZap size={14} />} Test connection
              </button>
              {test.data && <span className={test.data.ok ? 'form-ok' : 'form-error'}>{test.data.message}</span>}
              {test.error && <span className="form-error">{(test.error as Error).message}</span>}
            </div>
            {v.org && (
              <p className="muted">
                Last changed by {v.org.updatedBy || 'an admin'} on {new Date(v.org.updatedAt).toLocaleString()}.
              </p>
            )}
          </div>
        )}
      </section>
    </div>
  );
}
