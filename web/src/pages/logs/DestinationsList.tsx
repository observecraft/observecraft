import { useState } from 'react';
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { AlertTriangle, Plus, Search, Send, Workflow, X } from 'lucide-react';
import {
  createDestination,
  deleteDestination,
  listDestinations,
  setDestinationEnabled,
  testDestination,
  updateDestination,
  type CustomDestination,
  type CustomDestinationInput,
} from '../../api/logsConfig/client';
import { BrandLogo, hasBrandLogo } from '../../components/brand/BrandLogo';
import { ErrorState, Loading } from '../../components/ui/AsyncState';
import { Menu } from '../../components/ui/Dropdown';
import { Modal } from '../../components/ui/Modal';
import { compact, timeAgo } from '../../lib/format';
import { useProfile } from '../settings/queries';
import { Columns, FilterQuery, FlowStep, Num, SectionHead } from './config/configUi';

const KEY = ['log-custom-destinations'];

type Kind = 'http' | 'splunk_hec' | 'elasticsearch' | 'microsoft_sentinel';
const KINDS: { kind: Kind; label: string; logo: string }[] = [
  { kind: 'http', label: 'HTTP', logo: '' },
  { kind: 'splunk_hec', label: 'Splunk HTTP Event Collector', logo: 'splunk' },
  { kind: 'elasticsearch', label: 'Elasticsearch', logo: 'elasticsearch' },
  { kind: 'microsoft_sentinel', label: 'Microsoft Sentinel', logo: 'azure' },
];
const kindOf = (d: CustomDestination) => String(d.destination.type ?? 'http') as Kind;
const kindLabel = (k: string) => KINDS.find((x) => x.kind === k)?.label ?? k;

function KindLogo({ kind }: { kind: string }) {
  const logo = KINDS.find((x) => x.kind === kind)?.logo;
  return logo && hasBrandLogo(logo) ? <BrandLogo name={logo} size={14} maxRatio={3.4} title={kindLabel(kind)} /> : <Send size={14} className="muted" />;
}

/** Archiving & Forwarding → Custom Destinations: logs matching a query are forwarded as they are processed. */
export function DestinationsList() {
  const qc = useQueryClient();
  const me = useProfile();
  const isAdmin = me.data?.roles.some((r) => r.id === `${me.data?.org.id}-admin`) ?? false;
  const q = useQuery({ queryKey: KEY, queryFn: listDestinations, refetchInterval: 30_000 });
  const [editing, setEditing] = useState<CustomDestination | 'new' | null>(null);
  const [search, setSearch] = useState('');
  const [open, setOpen] = useState(true);
  const refresh = () => qc.invalidateQueries({ queryKey: KEY });
  const toggle = useMutation({ mutationFn: (d: CustomDestination) => setDestinationEnabled(d.id, !d.enabled), onSuccess: refresh });
  const remove = useMutation({ mutationFn: deleteDestination, onSuccess: refresh });
  const test = useMutation({ mutationFn: testDestination });

  if (q.isPending) return <Loading />;
  if (q.isError) return <ErrorState error={q.error} onRetry={() => q.refetch()} />;
  const all = q.data;
  const needle = search.trim().toLowerCase();
  const shown = all.filter((d) => !needle || d.name.toLowerCase().includes(needle) || d.query.toLowerCase().includes(needle));
  const active = all.filter((d) => d.enabled).length;
  const template = '24px minmax(130px, 1fr) minmax(140px, 1.2fr) minmax(180px, 1.3fr) minmax(90px, 0.8fr) 140px 46px 24px';
  return (
    <>
      <div className="lc-title-row">
        <div className="lc-count">
          <b>{all.length}</b> custom destinations defined
        </div>
        {isAdmin && (
          <button className="btn btn-primary" onClick={() => setEditing('new')}>
            <Plus size={14} /> New Destination
          </button>
        )}
      </div>
      <div className="lc-search">
        <Search size={14} className="muted" />
        <input value={search} onChange={(e) => setSearch(e.target.value)} placeholder="Filter Custom Destinations" aria-label="Filter custom destinations" />
      </div>
      <div className="lc-flow">
        <div className="lc-flow-box">
          <FlowStep icon={<Workflow size={15} />} label="Pipelines" to="/logs/pipelines" right="Processed logs are forwarded whether an index keeps them or not" last />
        </div>
        <div className="lc-flow-link" />
      </div>
      {(toggle.error || remove.error) && <p className="form-error">{((toggle.error ?? remove.error) as Error).message}</p>}
      {test.data && <p className={test.data.ok ? 'ar-note' : 'form-error'}>{test.data.ok ? 'The destination accepted the test log.' : test.data.error}</p>}
      <section className="lc-section">
        <SectionHead open={open} onToggle={() => setOpen((o) => !o)} label="NAME">
          <span>
            <b>{active}</b> ACTIVE
          </span>
          <span className="sep">|</span>
          <span>
            <b>{all.length - active}</b> DISABLED
          </span>
        </SectionHead>
        {open && (
          <>
            <Columns template={template} cols={['', 'Name', 'Filters', 'Destination', 'Tags', 'Forwarded', 'Enabled', '']} />
            {shown.map((d) => {
              const kind = kindOf(d);
              const endpoint = String(d.destination.endpoint ?? d.destination.data_collection_endpoint ?? '');
              return (
                <div key={d.id} className={`lc-row${d.enabled ? '' : ' dim'}`} style={{ gridTemplateColumns: template }}>
                  <Num n={all.indexOf(d) + 1} muted={!d.enabled} />
                  <span className="lc-cell-name">
                    <button onClick={() => isAdmin && setEditing(d)} title={d.name}>
                      {d.name}
                    </button>
                  </span>
                  <FilterQuery query={d.query} />
                  <span className="lc-cell-name" title={endpoint}>
                    <KindLogo kind={kind} />
                    <span>{kindLabel(kind)}</span>
                    <span className="muted mono">{endpoint.replace(/^https?:\/\//, '')}</span>
                  </span>
                  <span className="muted">{!d.forwardTags ? 'None' : d.tagsList.length === 0 ? 'All' : `${d.tagsListType === 'ALLOW_LIST' ? 'Only' : 'All but'} ${d.tagsList.join(', ')}`}</span>
                  <span className="lc-keep">
                    {d.lastError ? (
                      <span className="ar-status bad" title={d.lastError}>
                        <AlertTriangle size={13} /> {d.lastErrorAt ? `Failing ${timeAgo(d.lastErrorAt)}` : 'Not forwarded'}
                      </span>
                    ) : (
                      <span>{compact(d.forwarded)} logs</span>
                    )}
                  </span>
                  <span className="lc-keep">
                    <button
                      role="switch"
                      aria-checked={d.enabled}
                      aria-label={d.enabled ? `Disable ${d.name}` : `Enable ${d.name}`}
                      className={`switch${d.enabled ? ' on' : ''}`}
                      disabled={!isAdmin || toggle.isPending}
                      onClick={() => toggle.mutate(d)}
                    />
                  </span>
                  <span className="lc-keep">
                    {isAdmin && (
                      <Menu
                        trigger={<span aria-label="Destination actions">⋯</span>}
                        triggerClassName="icon-plain"
                        items={[
                          { label: 'Edit', onClick: () => setEditing(d) },
                          { label: test.isPending ? 'Sending…' : 'Send a test log', onClick: () => test.mutate(d.id), disabled: test.isPending },
                          { label: 'Delete', onClick: () => window.confirm(`Delete "${d.name}"? Logs stop being forwarded there.`) && remove.mutate(d.id) },
                        ]}
                      />
                    )}
                  </span>
                </div>
              );
            })}
            {all.length === 0 && <p className="lc-empty">No custom destinations: logs stay in ObserveCraft and its archives.</p>}
            {all.length > 0 && shown.length === 0 && <p className="lc-empty">No destination matches.</p>}
          </>
        )}
      </section>
      <p className="muted lc-note">
        Logs are forwarded after the pipelines, in batches every few seconds, shaped as the Datadog forwarder sends them (<code>message</code>, <code>status</code>, <code>service</code>,{' '}
        <code>ddsource</code>, <code>ddtags</code> and attributes). Microsoft Sentinel destinations are kept but not forwarded to yet: they need Azure credentials.
      </p>
      {editing && <DestinationModal destination={editing === 'new' ? null : editing} onClose={() => setEditing(null)} onSaved={() => (setEditing(null), void refresh())} />}
    </>
  );
}

function DestinationModal({ destination, onClose, onSaved }: { destination: CustomDestination | null; onClose: () => void; onSaved: () => void }) {
  const d = destination?.destination ?? {};
  const str = (k: string, from: Record<string, unknown> = d) => (typeof from[k] === 'string' ? (from[k] as string) : '');
  const auth = (d.auth ?? {}) as Record<string, unknown>;
  const [name, setName] = useState(destination?.name ?? '');
  const [query, setQuery] = useState(destination?.query ?? '');
  const [kind, setKind] = useState<Kind>(destination ? kindOf(destination) : 'http');
  const [endpoint, setEndpoint] = useState(str('endpoint'));
  const [authType, setAuthType] = useState(str('type', auth) || 'basic');
  const [user, setUser] = useState('');
  const [secret, setSecret] = useState('');
  const [header, setHeader] = useState(str('header_name', auth));
  const [sourcetype, setSourcetype] = useState(str('sourcetype'));
  const [index, setIndex] = useState(str('index_name'));
  const [rotation, setRotation] = useState(str('index_rotation'));
  const [sentinel, setSentinel] = useState({
    tenant_id: str('tenant_id'),
    client_id: str('client_id'),
    data_collection_endpoint: str('data_collection_endpoint'),
    data_collection_rule_id: str('data_collection_rule_id'),
    stream_name: str('stream_name'),
  });
  const [forwardTags, setForwardTags] = useState(destination?.forwardTags ?? true);
  const [listType, setListType] = useState(destination?.tagsListType ?? 'ALLOW_LIST');
  const [tags, setTags] = useState((destination?.tagsList ?? []).join(', '));
  const sameKind = destination && kindOf(destination) === kind;

  const body = (): CustomDestinationInput => {
    let dest: Record<string, unknown>;
    switch (kind) {
      case 'http':
        dest = {
          type: 'http',
          endpoint,
          auth:
            authType === 'custom_header'
              ? { type: 'custom_header', header_name: header, ...(secret || !sameKind ? { header_value: secret } : {}) }
              : { type: 'basic', ...(user || !sameKind ? { username: user } : {}), ...(secret || !sameKind ? { password: secret } : {}) },
        };
        break;
      case 'splunk_hec':
        dest = { type: 'splunk_hec', endpoint, ...(secret || !sameKind ? { access_token: secret } : {}), ...(sourcetype ? { sourcetype } : {}) };
        break;
      case 'elasticsearch':
        dest = {
          type: 'elasticsearch',
          endpoint,
          index_name: index,
          ...(rotation ? { index_rotation: rotation } : {}),
          auth: { ...(user || !sameKind ? { username: user } : {}), ...(secret || !sameKind ? { password: secret } : {}) },
        };
        break;
      default:
        dest = { type: 'microsoft_sentinel', ...sentinel };
    }
    return {
      name,
      query,
      enabled: destination?.enabled ?? true,
      forwardTags,
      tagsList: tags
        .split(',')
        .map((t) => t.trim())
        .filter(Boolean),
      tagsListType: listType,
      destination: dest,
    };
  };
  const save = useMutation({ mutationFn: () => (destination ? updateDestination(destination.id, body()) : createDestination(body())), onSuccess: onSaved });
  const keep = sameKind ? 'leave empty to keep the current one' : '';
  return (
    <Modal onClose={onClose} label={destination ? 'Edit destination' : 'New destination'} width={640} height="auto">
      <form
        className="small-modal ar-form"
        onSubmit={(e) => {
          e.preventDefault();
          save.mutate();
        }}
      >
        <div className="modal-title-row">
          <h3>{destination ? `Edit ${destination.name}` : 'New Custom Destination'}</h3>
          <button type="button" className="icon-plain" onClick={onClose} title="Close">
            <X size={18} />
          </button>
        </div>
        <label className="form-label">
          Name
          <input value={name} onChange={(e) => setName(e.target.value)} placeholder="Security team SIEM" maxLength={200} required autoFocus={!destination} />
        </label>
        <label className="form-label">
          Logs to forward
          <input className="mono" value={query} onChange={(e) => setQuery(e.target.value)} placeholder="every log (or e.g. source:cloudtrail status:error)" />
        </label>
        <label className="form-label">
          Destination
          <select value={kind} onChange={(e) => setKind(e.target.value as Kind)}>
            {KINDS.map((k) => (
              <option key={k.kind} value={k.kind}>
                {k.label}
              </option>
            ))}
          </select>
        </label>
        {kind !== 'microsoft_sentinel' && (
          <label className="form-label">
            Endpoint (https)
            <input
              value={endpoint}
              onChange={(e) => setEndpoint(e.target.value)}
              placeholder={kind === 'splunk_hec' ? 'https://splunk.example.com:8088' : 'https://siem.example.com/ingest'}
              required
            />
          </label>
        )}
        {kind === 'http' && (
          <div className="ar-grid">
            <label className="form-label">
              Authentication
              <select value={authType} onChange={(e) => setAuthType(e.target.value)}>
                <option value="basic">Basic</option>
                <option value="custom_header">Custom header</option>
              </select>
            </label>
            {authType === 'custom_header' ? (
              <>
                <label className="form-label">
                  Header name
                  <input value={header} onChange={(e) => setHeader(e.target.value)} placeholder="X-Api-Key" required />
                </label>
                <label className="form-label">
                  Header value
                  <input type="password" value={secret} onChange={(e) => setSecret(e.target.value)} placeholder={keep} autoComplete="new-password" />
                </label>
              </>
            ) : (
              <>
                <label className="form-label">
                  Username
                  <input value={user} onChange={(e) => setUser(e.target.value)} placeholder={keep} autoComplete="off" />
                </label>
                <label className="form-label">
                  Password
                  <input type="password" value={secret} onChange={(e) => setSecret(e.target.value)} placeholder={keep} autoComplete="new-password" />
                </label>
              </>
            )}
          </div>
        )}
        {kind === 'splunk_hec' && (
          <div className="ar-grid">
            <label className="form-label">
              HEC token
              <input type="password" value={secret} onChange={(e) => setSecret(e.target.value)} placeholder={keep} autoComplete="new-password" />
            </label>
            <label className="form-label">
              Source type
              <input value={sourcetype} onChange={(e) => setSourcetype(e.target.value)} placeholder="_json" />
            </label>
          </div>
        )}
        {kind === 'elasticsearch' && (
          <>
            <div className="ar-grid">
              <label className="form-label">
                Index
                <input value={index} onChange={(e) => setIndex(e.target.value)} placeholder="checkout-logs" required />
              </label>
              <label className="form-label">
                Index rotation
                <select value={rotation} onChange={(e) => setRotation(e.target.value)}>
                  <option value="">None</option>
                  <option value="yyyy-MM-dd">Daily (yyyy-MM-dd)</option>
                  <option value="yyyy-'W'ww">Weekly (yyyy-'W'ww)</option>
                  <option value="yyyy-MM">Monthly (yyyy-MM)</option>
                </select>
              </label>
            </div>
            <div className="ar-grid">
              <label className="form-label">
                Username
                <input value={user} onChange={(e) => setUser(e.target.value)} placeholder={keep} autoComplete="off" />
              </label>
              <label className="form-label">
                Password
                <input type="password" value={secret} onChange={(e) => setSecret(e.target.value)} placeholder={keep} autoComplete="new-password" />
              </label>
            </div>
          </>
        )}
        {kind === 'microsoft_sentinel' && (
          <>
            <div className="ar-grid">
              {(['tenant_id', 'client_id', 'stream_name'] as const).map((k) => (
                <label key={k} className="form-label">
                  {k === 'tenant_id' ? 'Tenant ID' : k === 'client_id' ? 'Client ID' : 'Stream name'}
                  <input value={sentinel[k]} onChange={(e) => setSentinel((s) => ({ ...s, [k]: e.target.value }))} required />
                </label>
              ))}
            </div>
            <div className="ar-grid">
              <label className="form-label">
                Data collection endpoint
                <input value={sentinel.data_collection_endpoint} onChange={(e) => setSentinel((s) => ({ ...s, data_collection_endpoint: e.target.value }))} required />
              </label>
              <label className="form-label">
                Data collection rule ID
                <input value={sentinel.data_collection_rule_id} onChange={(e) => setSentinel((s) => ({ ...s, data_collection_rule_id: e.target.value }))} required />
              </label>
            </div>
            <p className="muted">Kept for Terraform compatibility: logs aren't forwarded to Sentinel yet (it needs Azure credentials ObserveCraft doesn't hold).</p>
          </>
        )}
        <label className="ar-check">
          <input type="checkbox" checked={forwardTags} onChange={(e) => setForwardTags(e.target.checked)} /> Forward tags
        </label>
        {forwardTags && (
          <div className="ar-grid">
            <label className="form-label">
              Tag keys
              <select value={listType} onChange={(e) => setListType(e.target.value)}>
                <option value="ALLOW_LIST">Only these</option>
                <option value="BLOCK_LIST">All but these</option>
              </select>
            </label>
            <label className="form-label">
              Keys (comma-separated; empty = every tag)
              <input value={tags} onChange={(e) => setTags(e.target.value)} placeholder="env, service, team" />
            </label>
          </div>
        )}
        {save.error && <p className="form-error">{(save.error as Error).message}</p>}
        <div className="small-modal-actions">
          <button type="button" className="btn" onClick={onClose}>
            Cancel
          </button>
          <button className="btn btn-primary" disabled={save.isPending}>
            {destination ? 'Save' : 'Create Destination'}
          </button>
        </div>
      </form>
    </Modal>
  );
}
