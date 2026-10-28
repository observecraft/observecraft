import { Link } from 'react-router-dom';
import { useMemo, useState, type ReactNode } from 'react';
import { ArrowDown, ArrowUp, ChevronDown, Download, Flag, PanelLeftClose, PanelLeftOpen, PhoneCall, Plus, Search, Star, Trash2, X } from 'lucide-react';
import type { IncidentDetail, TimelineEvent } from '../../api/incidents/client';
import { ErrorState, Loading } from '../../components/ui/AsyncState';
import { Modal } from '../../components/ui/Modal';
import { UserAvatar } from '../../components/ui/UserAvatar';
import { timeAgo } from '../../lib/format';
import { useProfile, useTeams, useUsers } from '../settings/queries';
import {
  fmtDateTime,
  useAddDocument,
  useAddFollowup,
  useAddNote,
  useAddResponder,
  useCreateNotification,
  useCreatePage,
  useDocuments,
  useGeneratePostmortem,
  useNotifications,
  usePages,
  usePostmortem,
  useRemoveDocument,
  useRemoveResponder,
  useResponders,
  useSavePostmortem,
  useSetImportant,
  useTimeline,
  useToggleFollowup,
  useUpdatePage,
} from './queries';

const URL_RE = /(https?:\/\/[^\s<>"')]+)/g;
const HAS_URL = /https?:\/\//;

/** Plain text with http(s) URLs as links. Never renders HTML from the text. */
export function Linkify({ text }: { text: string }) {
  const parts = text.split(URL_RE);
  return (
    <>
      {parts.map((p, i) =>
        i % 2 === 1 ? (
          <a key={i} href={p} target="_blank" rel="noopener noreferrer">
            {p}
          </a>
        ) : (
          <span key={i}>{p}</span>
        ),
      )}
    </>
  );
}

function errorText(e: unknown) {
  return e instanceof Error ? e.message : 'Something went wrong';
}

// ================================================================ timeline

type ContentType = 'Status Update' | 'Integration Update' | 'Responder Note' | 'Link' | 'Chat Message';
const CONTENT_TYPES: ContentType[] = ['Status Update', 'Integration Update', 'Responder Note', 'Link', 'Chat Message'];

function contentType(e: TimelineEvent): ContentType {
  if (['declared', 'status', 'severity', 'commander'].includes(e.kind)) return 'Status Update';
  if (['notification', 'impact', 'followup'].includes(e.kind)) return 'Integration Update';
  return HAS_URL.test(e.body) ? 'Link' : 'Responder Note';
}

function toggle<T>(set: Set<T>, v: T): Set<T> {
  const n = new Set(set);
  if (!n.delete(v)) n.add(v);
  return n;
}

function FilterGroup({ title, children }: { title: string; children: ReactNode }) {
  const [open, setOpen] = useState(true);
  return (
    <div className="itl-group">
      <button className="itl-group-title" onClick={() => setOpen((o) => !o)}>
        <ChevronDown size={16} className={open ? '' : 'rot-90'} /> {title}
      </button>
      {open && children}
    </div>
  );
}

export function TimelineTab({ inc }: { inc: IncidentDetail }) {
  const q = useTimeline(inc.id);
  const me = useProfile();
  const add = useAddNote(inc.id);
  const star = useSetImportant(inc.id);
  const [note, setNote] = useState('');
  const [controls, setControls] = useState(true);
  const [newest, setNewest] = useState(false);
  const [hiddenTypes, setHiddenTypes] = useState<Set<ContentType>>(new Set());
  const [hiddenImportance, setHiddenImportance] = useState<Set<'Yes' | 'No'>>(new Set());
  const [hiddenAuthors, setHiddenAuthors] = useState<Set<string>>(new Set());

  const events = q.data ?? [];
  const counts = useMemo(() => {
    const types = new Map<ContentType, number>();
    const authors = new Map<string, number>();
    let yes = 0;
    for (const e of events) {
      types.set(contentType(e), (types.get(contentType(e)) ?? 0) + 1);
      authors.set(e.author, (authors.get(e.author) ?? 0) + 1);
      if (e.important) yes++;
    }
    return { types, authors, yes, no: events.length - yes };
  }, [events]);
  const shown = events
    .filter((e) => !hiddenTypes.has(contentType(e)) && !hiddenImportance.has(e.important ? 'Yes' : 'No') && !hiddenAuthors.has(e.author))
    .sort((a, b) => (newest ? b.createdAt - a.createdAt : a.createdAt - b.createdAt));
  const submit = () => note.trim() && add.mutate(note, { onSuccess: () => setNote('') });

  return (
    <section className="settings-card itl-card">
      <div className="itl-compose">
        {me.data && <UserAvatar id={me.data.id} name={me.data.name} size={44} />}
        <textarea
          rows={1}
          placeholder="Write a note or paste a link. Press Cmd + Return to submit."
          value={note}
          onChange={(e) => setNote(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === 'Enter' && (e.metaKey || e.ctrlKey)) {
              e.preventDefault();
              submit();
            }
          }}
        />
        <button className="btn btn-primary itl-add" disabled={!note.trim() || add.isPending} onClick={submit}>
          <Plus size={16} /> Add Note
        </button>
      </div>
      {add.isError && <p className="form-error">{errorText(add.error)}</p>}
      <div className="itl-bar">
        <button className="link-btn" onClick={() => setControls((c) => !c)}>
          {controls ? <PanelLeftClose size={16} /> : <PanelLeftOpen size={16} />} {controls ? 'Hide Controls' : 'Show Controls'}
        </button>
        <span className="muted">{shown.length} entries</span>
        <button className="btn" onClick={() => setNewest((n) => !n)}>
          {newest ? 'Newest First' : 'Oldest First'} {newest ? <ArrowUp size={14} /> : <ArrowDown size={14} />}
        </button>
      </div>
      <div className="itl-layout">
        {controls && (
          <aside className="itl-filters">
            <div className="inc-group-title">FILTERS</div>
            <FilterGroup title="Content Type">
              {CONTENT_TYPES.map((t) => (
                <label key={t} className="inc-check">
                  <input type="checkbox" className="checkbox" checked={!hiddenTypes.has(t)} onChange={() => setHiddenTypes(toggle(hiddenTypes, t))} /> {t}
                  <span className="inc-count">{counts.types.get(t) ?? 0}</span>
                </label>
              ))}
            </FilterGroup>
            <FilterGroup title="Important">
              {(['Yes', 'No'] as const).map((v) => (
                <label key={v} className="inc-check">
                  <input type="checkbox" className="checkbox" checked={!hiddenImportance.has(v)} onChange={() => setHiddenImportance(toggle(hiddenImportance, v))} /> {v}
                  <span className="inc-count">{v === 'Yes' ? counts.yes : counts.no}</span>
                </label>
              ))}
            </FilterGroup>
            <FilterGroup title="Responder">
              {[...counts.authors].map(([a, n]) => (
                <label key={a} className="inc-check">
                  <input type="checkbox" className="checkbox" checked={!hiddenAuthors.has(a)} onChange={() => setHiddenAuthors(toggle(hiddenAuthors, a))} />
                  <span className="ellipsis">{a}</span>
                  <span className="inc-count">{n}</span>
                </label>
              ))}
            </FilterGroup>
          </aside>
        )}
        <div className="itl-entries">
          {q.isPending ? (
            <Loading />
          ) : q.isError ? (
            <ErrorState error={q.error} onRetry={() => q.refetch()} />
          ) : shown.length === 0 ? (
            <p className="muted">No entries match the filters.</p>
          ) : (
            shown.map((e, i) => {
              const prev = shown[i - 1];
              const newDay = !prev || new Date(prev.createdAt).toDateString() !== new Date(e.createdAt).toDateString();
              const time = new Date(e.createdAt).toLocaleTimeString('en-US', { hour: 'numeric', minute: '2-digit' }).toLowerCase();
              return (
                <div key={e.id} className={`itl-entry itle-${e.kind}`}>
                  <div className="itl-when">
                    {newDay && <b>{new Date(e.createdAt).toLocaleDateString('en-US', { month: 'short', day: 'numeric', year: 'numeric' })}</b>}
                    <span>{time}</span>
                  </div>
                  <div className="itl-rail">{e.kind === 'declared' ? <Flag size={16} className="itl-flag" /> : <i className="itl-dot" />}</div>
                  <div className="itl-content">
                    {e.kind === 'declared' ? (
                      <>
                        <div className="itl-head">
                          <b>Incident declared</b> <span className="inc-status inc-active">ACTIVE</span> <span className="muted">by {e.author}</span>
                        </div>
                        <div>
                          IR-{inc.id}: {inc.title}
                        </div>
                        <div className="itl-props">
                          <span className="muted">Severity</span> <span className="prop-chip">{e.body.match(/SEV-\d|UNKNOWN/)?.[0] ?? inc.severity}</span>
                        </div>
                        {inc.commander && (
                          <div className="itl-props">
                            <span className="muted">Incident Commander</span>
                            <span className="person-chip">
                              <UserAvatar id={inc.commander.id} name={inc.commander.name} size={18} /> {inc.commander.name}
                            </span>
                          </div>
                        )}
                      </>
                    ) : (
                      <div className="itl-note">
                        <UserAvatar id={e.authorId ?? e.author} name={e.author} size={40} />
                        <div>
                          <div className="itl-head">
                            <b>{e.author}</b> <span className="muted">· {contentType(e)}</span>
                          </div>
                          <div className="itl-text">
                            <Linkify text={e.body} />
                          </div>
                        </div>
                      </div>
                    )}
                  </div>
                  <button
                    className={`itl-star${e.important ? ' on' : ''}`}
                    title={e.important ? 'Unmark important' : 'Mark important'}
                    onClick={() => star.mutate({ eventId: e.id, important: !e.important })}
                  >
                    <Star size={16} />
                  </button>
                </div>
              );
            })
          )}
        </div>
      </div>
    </section>
  );
}

// ================================================================ post-incident

/** Minimal Markdown → React (headings, tables, task/bullet lists, bold, italics, links). No HTML passthrough. */
function Markdown({ source }: { source: string }) {
  const inline = (t: string, key: string): ReactNode[] => {
    const out: ReactNode[] = [];
    const re = /(\*\*[^*]+\*\*|_[^_]+_|\[[^\]]+\]\(https?:\/\/[^)\s]+\))/g;
    let last = 0;
    let m: RegExpExecArray | null;
    let i = 0;
    while ((m = re.exec(t))) {
      if (m.index > last) out.push(<Linkify key={`${key}t${i++}`} text={t.slice(last, m.index)} />);
      const tok = m[0];
      if (tok.startsWith('**')) out.push(<b key={`${key}b${i++}`}>{tok.slice(2, -2)}</b>);
      else if (tok.startsWith('_')) out.push(<em key={`${key}i${i++}`}>{tok.slice(1, -1)}</em>);
      else {
        const [, label, href] = /\[([^\]]+)\]\(([^)]+)\)/.exec(tok)!;
        out.push(
          <a key={`${key}a${i++}`} href={href} target="_blank" rel="noopener noreferrer">
            {label}
          </a>,
        );
      }
      last = m.index + tok.length;
    }
    if (last < t.length) out.push(<Linkify key={`${key}e`} text={t.slice(last)} />);
    return out;
  };
  const lines = source.split('\n');
  const blocks: ReactNode[] = [];
  for (let i = 0; i < lines.length; i++) {
    const l = lines[i];
    if (l.startsWith('# ')) blocks.push(<h2 key={i}>{inline(l.slice(2), `${i}`)}</h2>);
    else if (l.startsWith('## ')) blocks.push(<h3 key={i}>{inline(l.slice(3), `${i}`)}</h3>);
    else if (l.startsWith('### ')) blocks.push(<h4 key={i}>{inline(l.slice(4), `${i}`)}</h4>);
    else if (l.startsWith('|')) {
      const rows: string[][] = [];
      while (i < lines.length && lines[i].startsWith('|')) {
        const cells = lines[i]
          .slice(1, -1)
          .split(/(?<!\\)\|/)
          .map((c) => c.trim().replace(/\\\|/g, '|'));
        if (!cells.every((c) => /^-*$/.test(c))) rows.push(cells);
        i++;
      }
      i--;
      blocks.push(
        <table key={i} className="pm-table">
          <tbody>
            {rows
              .filter((r) => r.some(Boolean))
              .map((r, j) => (
                <tr key={j}>
                  {r.map((c, k) => (
                    <td key={k}>{inline(c, `${i}-${j}-${k}`)}</td>
                  ))}
                </tr>
              ))}
          </tbody>
        </table>,
      );
    } else if (l.startsWith('- ')) {
      const items: string[] = [];
      while (i < lines.length && lines[i].startsWith('- ')) items.push(lines[i++].slice(2));
      i--;
      blocks.push(
        <ul key={i} className="pm-list">
          {items.map((it, j) => {
            const task = /^\[( |x)\] /.exec(it);
            return (
              <li key={j} className={task ? 'pm-task' : ''}>
                {task && <input type="checkbox" className="checkbox" checked={task[1] === 'x'} readOnly />}
                {inline(task ? it.slice(4) : it, `${i}-${j}`)}
              </li>
            );
          })}
        </ul>,
      );
    } else if (l.trim()) blocks.push(<p key={i}>{inline(l, `${i}`)}</p>);
  }
  return <div className="pm-doc">{blocks}</div>;
}

const TEMPLATE_PREVIEW: [string, string | null, string][] = [
  ['Overview', null, 'Incident properties and overview will be displayed here'],
  ['What Happened?', 'Impact on Customers', 'Summary and recorded impacts'],
  ['Why Did it Happen?', 'Root Cause', 'The root cause from the incident overview'],
  ['Timeline', null, 'The incident timeline will be embedded here'],
  ['How do we prevent it in the future?', 'Action Items', 'Follow-ups from this incident will be listed here'],
  ['Lessons Learned', null, 'To be completed by the team'],
];

export function PostIncidentTab({ inc }: { inc: IncidentDetail }) {
  const pm = usePostmortem(inc.id);
  const generate = useGeneratePostmortem(inc.id);
  const save = useSavePostmortem(inc.id);
  const [template, setTemplate] = useState<'standard' | 'brief'>('standard');
  const [editing, setEditing] = useState<string | null>(null);
  const doc = pm.data ?? null;

  const download = () => {
    if (!doc) return;
    const a = document.createElement('a');
    a.href = URL.createObjectURL(new Blob([doc.content], { type: 'text/markdown' }));
    a.download = `postmortem-IR-${inc.id}.md`;
    a.click();
    URL.revokeObjectURL(a.href);
  };

  return (
    <div className="pi-layout">
      <section className="settings-card pi-main">
        <div className="pi-bar">
          <label className="pi-field">
            Postmortem Template <span className="req">*</span>
            <select className="settings-input" value={template} onChange={(e) => setTemplate(e.target.value as 'standard' | 'brief')}>
              <option value="standard">ObserveCraft Template</option>
              <option value="brief">Brief Template</option>
            </select>
          </label>
          <span className="pi-field">
            Location <span className="pill-chip">Stored with the incident</span>
          </span>
          <span className="pi-actions">
            {doc && (
              <>
                <button className="btn" onClick={download}>
                  <Download size={14} /> Markdown
                </button>
                <button className="btn" onClick={() => setEditing(editing === null ? doc.content : null)}>
                  {editing === null ? 'Edit' : 'Cancel edit'}
                </button>
              </>
            )}
            <button
              className="btn btn-primary"
              disabled={generate.isPending}
              onClick={() => (!doc || window.confirm('Regenerate the postmortem? Manual edits will be replaced.')) && generate.mutate(template)}
            >
              {doc ? 'Regenerate' : 'Generate Postmortem'}
            </button>
          </span>
        </div>
        {(generate.error || save.error) && <p className="form-error">{errorText(generate.error ?? save.error)}</p>}
        {pm.isPending ? (
          <Loading />
        ) : pm.isError ? (
          <ErrorState error={pm.error} onRetry={() => pm.refetch()} />
        ) : editing !== null ? (
          <div className="pi-edit">
            <textarea value={editing} onChange={(e) => setEditing(e.target.value)} rows={28} />
            <button className="btn btn-primary" disabled={save.isPending} onClick={() => save.mutate(editing, { onSuccess: () => setEditing(null) })}>
              Save
            </button>
          </div>
        ) : doc ? (
          <>
            <p className="muted pi-meta">
              Generated by {doc.createdBy} · updated {timeAgo(doc.updatedAt)}
            </p>
            <Markdown source={doc.content} />
          </>
        ) : (
          <div className="pi-preview">
            <div className="pi-preview-title">Template Preview</div>
            <h3>
              Postmortem IR-{inc.id}: {inc.title}
            </h3>
            {TEMPLATE_PREVIEW.filter(([h]) => template === 'standard' || !['Timeline', 'Lessons Learned'].includes(h)).map(([h, sub, body]) => (
              <div key={h} className="pi-section">
                <h4>{h}</h4>
                {sub && <div className="muted">{sub}</div>}
                <em>{body}</em>
              </div>
            ))}
          </div>
        )}
      </section>
      <div className="pi-side">
        <FollowupsCard inc={inc} />
        <DocumentsCard incidentId={inc.id} />
      </div>
    </div>
  );
}

function FollowupsCard({ inc }: { inc: IncidentDetail }) {
  const add = useAddFollowup(inc.id);
  const toggleDone = useToggleFollowup(inc.id);
  const [draft, setDraft] = useState<string | null>(null);
  return (
    <section className="settings-card pi-card">
      <header>
        <b>Follow-Ups</b>
        <button className="btn" onClick={() => setDraft('')}>
          <Plus size={14} /> Add Follow-Up
        </button>
      </header>
      {inc.followups.length === 0 && draft === null && <p className="muted pi-empty">No follow-ups related to this incident</p>}
      {inc.followups.map((f) => (
        <label key={f.id} className="inc-check">
          <input type="checkbox" className="checkbox" checked={f.done} onChange={() => toggleDone.mutate({ followupId: f.id, done: !f.done })} />
          <span className={f.done ? 'done' : ''}>{f.title}</span>
        </label>
      ))}
      {draft !== null && (
        <form className="inc-edit" onSubmit={(e) => (e.preventDefault(), draft.trim() && add.mutate(draft, { onSuccess: () => setDraft(null) }))}>
          <input value={draft} onChange={(e) => setDraft(e.target.value)} placeholder="What needs to happen?" autoFocus />
        </form>
      )}
    </section>
  );
}

function DocumentsCard({ incidentId }: { incidentId: number }) {
  const q = useDocuments(incidentId);
  const add = useAddDocument(incidentId);
  const remove = useRemoveDocument(incidentId);
  const [form, setForm] = useState<{ title: string; url: string } | null>(null);
  return (
    <section className="settings-card pi-card">
      <header>
        <b>Documents</b>
        <button className="btn" onClick={() => setForm({ title: '', url: '' })}>
          <Plus size={14} /> Add Links
        </button>
      </header>
      {(q.data ?? []).length === 0 && !form && <p className="muted pi-empty">No document is linked to this incident</p>}
      {(q.data ?? []).map((d) => (
        <div key={d.id} className="pi-doc">
          <a href={d.url} target="_blank" rel="noopener noreferrer" className="ellipsis">
            {d.title}
          </a>
          <button className="icon-plain" title="Remove link" onClick={() => remove.mutate(d.id)}>
            <Trash2 size={14} />
          </button>
        </div>
      ))}
      {form && (
        <form className="inc-edit" onSubmit={(e) => (e.preventDefault(), add.mutate(form, { onSuccess: () => setForm(null) }))}>
          <input placeholder="Title" value={form.title} onChange={(e) => setForm({ ...form, title: e.target.value })} required autoFocus />
          <input placeholder="https://…" value={form.url} onChange={(e) => setForm({ ...form, url: e.target.value })} required />
          <div className="form-actions">
            <button type="button" className="btn" onClick={() => setForm(null)}>
              Cancel
            </button>
            <button className="btn btn-primary" disabled={add.isPending}>
              Add
            </button>
          </div>
        </form>
      )}
      {add.isError && <p className="form-error">{errorText(add.error)}</p>}
    </section>
  );
}

// ================================================================ response team

const ROLE_LABEL: Record<string, string> = { incident_commander: 'Incident Commander', responder: 'Responder', communications_lead: 'Communications Lead', scribe: 'Scribe' };
const ACTIVE_MS = 60 * 60e3;

export function ResponseTeamTab({ inc }: { inc: IncidentDetail }) {
  const q = useResponders(inc.id);
  const users = useUsers({ query: '', statuses: ['active'], offset: 0, limit: 200 });
  const add = useAddResponder(inc.id);
  const remove = useRemoveResponder(inc.id);
  const [search, setSearch] = useState('');
  const [adding, setAdding] = useState<{ userId: string; role: string } | null>(null);
  const members = (q.data ?? []).filter((m) => m.name.toLowerCase().includes(search.trim().toLowerCase()));
  const active = (q.data ?? []).filter((m) => m.lastActiveAt && Date.now() - m.lastActiveAt < ACTIVE_MS).length;
  return (
    <section className="settings-card">
      <div className="rt-head">
        <div>
          <h2>
            Response Team <span className="muted">({active} active)</span>
          </h2>
          <label className="settings-search small rt-search">
            <Search size={15} />
            <input placeholder="Search" value={search} onChange={(e) => setSearch(e.target.value)} />
          </label>
        </div>
        <button className="btn btn-primary" onClick={() => setAdding({ userId: '', role: 'responder' })}>
          <Plus size={14} /> Add Teammate
        </button>
      </div>
      {adding && (
        <form className="security-form" onSubmit={(e) => (e.preventDefault(), add.mutate(adding, { onSuccess: () => setAdding(null) }))}>
          <select className="settings-input" value={adding.userId} onChange={(e) => setAdding({ ...adding, userId: e.target.value })} required>
            <option value="">Select a teammate…</option>
            {(users.data?.users ?? [])
              .filter((u) => !(q.data ?? []).some((m) => m.userId === u.id))
              .map((u) => (
                <option key={u.id} value={u.id}>
                  {u.name}
                </option>
              ))}
          </select>
          <select className="settings-input" value={adding.role} onChange={(e) => setAdding({ ...adding, role: e.target.value })}>
            {['responder', 'communications_lead', 'scribe'].map((r) => (
              <option key={r} value={r}>
                {ROLE_LABEL[r]}
              </option>
            ))}
          </select>
          <button className="btn btn-primary" disabled={!adding.userId || add.isPending}>
            Add
          </button>
          <button type="button" className="btn" onClick={() => setAdding(null)}>
            Cancel
          </button>
        </form>
      )}
      {(add.error || remove.error) && <p className="form-error">{errorText(add.error ?? remove.error)}</p>}
      {q.isPending ? (
        <Loading />
      ) : q.isError ? (
        <ErrorState error={q.error} />
      ) : (
        <table className="settings-table">
          <thead>
            <tr>
              <th>NAME</th>
              <th>RESPONDER ROLE</th>
              <th>LAST ACTIVE</th>
              <th>ADDED</th>
              <th />
            </tr>
          </thead>
          <tbody>
            {members.map((m) => (
              <tr key={m.userId}>
                <td>
                  <span className="person-chip">
                    <UserAvatar id={m.userId} name={m.name} size={22} /> {m.name}
                  </span>
                </td>
                <td>
                  <span className="prop-chip">{ROLE_LABEL[m.role] ?? m.role}</span>
                </td>
                <td>{m.lastActiveAt && Date.now() - m.lastActiveAt < ACTIVE_MS ? <b className="tone-green">Active</b> : m.lastActiveAt ? `Active ${timeAgo(m.lastActiveAt)}` : 'Inactive'}</td>
                <td>{fmtDateTime(m.addedAt, false)}</td>
                <td className="num">
                  {m.role !== 'incident_commander' && (
                    <button className="icon-plain" title="Remove from response team" onClick={() => remove.mutate(m.userId)}>
                      <X size={16} />
                    </button>
                  )}
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      )}
    </section>
  );
}

// ================================================================ pages

export function PagesTab({ inc }: { inc: IncidentDetail }) {
  const q = usePages(inc.id);
  const teams = useTeams({ query: '', sort: 'name', offset: 0, limit: 200 });
  const create = useCreatePage(inc.id);
  const update = useUpdatePage(inc.id);
  const [team, setTeam] = useState('*');
  const [paging, setPaging] = useState<{ team: string; title: string } | null>(null);
  const rows = (q.data ?? []).filter((p) => team === '*' || p.team === team);
  return (
    <section className="settings-card">
      <div className="pg-bar">
        <span className="pg-filter">
          <span>Teams</span>
          <select value={team} onChange={(e) => setTeam(e.target.value)}>
            <option value="*">*</option>
            {(teams.data?.teams ?? []).map((t) => (
              <option key={t.handle} value={t.handle}>
                {t.handle}
              </option>
            ))}
          </select>
        </span>
        <button className="btn" onClick={() => setPaging({ team: inc.teams[0] ?? '', title: `IR-${inc.id}: ${inc.title}` })}>
          <PhoneCall size={14} /> Page
        </button>
      </div>
      {(create.error || update.error) && <p className="form-error">{errorText(create.error ?? update.error)}</p>}
      <table className="settings-table bordered pg-table">
        <thead>
          <tr>
            <th>PAGE</th>
            <th>STATUS</th>
            <th>PAGED TO</th>
            <th>RESPONDERS</th>
            <th>CREATION DATE</th>
            <th />
          </tr>
        </thead>
        <tbody>
          {rows.length === 0 ? (
            <tr>
              <td colSpan={6}>
                <div className="pg-empty">
                  <PhoneCall size={56} />
                  <span>No Pages found</span>
                </div>
              </td>
            </tr>
          ) : (
            rows.map((p) => (
              <tr key={p.id}>
                <td className="ellipsis pg-title">{p.title}</td>
                <td>
                  <span className={`pg-status pg-${p.status}`}>{p.status.toUpperCase()}</span>
                </td>
                <td>@{p.team}</td>
                <td>{p.acknowledgedBy ?? <span className="muted">—</span>}</td>
                <td>{fmtDateTime(p.createdAt)}</td>
                <td className="num">
                  {p.status === 'triggered' && (
                    <button className="btn btn-sm" onClick={() => update.mutate({ pageId: p.id, status: 'acknowledged' })}>
                      Acknowledge
                    </button>
                  )}{' '}
                  {p.status !== 'resolved' && (
                    <button className="btn btn-sm" onClick={() => update.mutate({ pageId: p.id, status: 'resolved' })}>
                      Resolve
                    </button>
                  )}
                </td>
              </tr>
            ))
          )}
        </tbody>
      </table>
      {paging && (
        <Modal onClose={() => setPaging(null)} label="Page a team" width={560} height="auto">
          <form className="small-modal" onSubmit={(e) => (e.preventDefault(), create.mutate(paging, { onSuccess: () => setPaging(null) }))}>
            <div className="modal-title-row">
              <h3>Page a team</h3>
              <button type="button" className="icon-plain" onClick={() => setPaging(null)} title="Close">
                <X size={18} />
              </button>
            </div>
            <label className="form-label">
              Team
              <select className="settings-input" value={paging.team} onChange={(e) => setPaging({ ...paging, team: e.target.value })} required>
                <option value="">Select a team…</option>
                {(teams.data?.teams ?? []).map((t) => (
                  <option key={t.handle} value={t.handle}>
                    {t.name}
                  </option>
                ))}
              </select>
            </label>
            <label className="form-label">
              Message
              <input value={paging.title} onChange={(e) => setPaging({ ...paging, title: e.target.value })} required maxLength={300} />
            </label>
            <p className="muted">
              The page goes to <Link to="/on-call/pages">On-Call</Link>: the team&apos;s escalation policy notifies whoever is on call by email, Slack or webhook (no phone or SMS).
            </p>
            <div className="small-modal-actions">
              <button type="button" className="btn" onClick={() => setPaging(null)}>
                Cancel
              </button>
              <button className="btn btn-primary" disabled={!paging.team || create.isPending}>
                Page
              </button>
            </div>
          </form>
        </Modal>
      )}
    </section>
  );
}

// ================================================================ notifications

export function NotificationsTab({ inc }: { inc: IncidentDetail }) {
  const q = useNotifications(inc.id);
  const create = useCreateNotification(inc.id);
  const [mode, setMode] = useState<'all' | 'manual' | 'automated'>('all');
  const [search, setSearch] = useState('');
  const [draft, setDraft] = useState<{ recipients: string; subject: string; body: string } | null>(null);
  const s = search.trim().toLowerCase();
  const rows = (q.data ?? []).filter(
    (n) => (mode === 'all' || (mode === 'automated') === n.automated) && (!s || [n.subject, n.body, n.author, ...n.recipients].some((v) => v.toLowerCase().includes(s))),
  );
  const status = inc.status[0].toUpperCase() + inc.status.slice(1);
  return (
    <section className="settings-card">
      <div className="nt-bar">
        <h2>Sent ({q.data?.length ?? 0})</h2>
        <label className="settings-search small nt-search">
          <Search size={15} />
          <input placeholder="Search all sent notifications" value={search} onChange={(e) => setSearch(e.target.value)} />
        </label>
        <span className="nt-right">
          <span className="apm-seg">
            {(['all', 'manual', 'automated'] as const).map((m) => (
              <button key={m} className={mode === m ? 'active' : ''} onClick={() => setMode(m)}>
                {m[0].toUpperCase() + m.slice(1)}
              </button>
            ))}
          </span>
          <button
            className="btn btn-primary"
            onClick={() =>
              setDraft({
                recipients: '',
                subject: `Incident #${inc.id}: ${inc.title}`,
                body: `Status: ${status} | Severity: ${inc.severity} | Commander: ${inc.commander?.name ?? 'Unassigned'}`,
              })
            }
          >
            <Plus size={14} /> New Notification
          </button>
        </span>
      </div>
      {q.isPending ? (
        <Loading />
      ) : q.isError ? (
        <ErrorState error={q.error} />
      ) : (
        <table className="settings-table nt-table">
          <thead>
            <tr>
              <th>RECIPIENTS</th>
              <th>MESSAGE</th>
              <th>LAST NOTIFIED</th>
              <th>AUTHOR</th>
              <th>MANUAL / AUTOMATED</th>
            </tr>
          </thead>
          <tbody>
            {rows.length === 0 ? (
              <tr>
                <td colSpan={5} className="muted center">
                  No notifications
                </td>
              </tr>
            ) : (
              rows.map((n) => (
                <tr key={n.id}>
                  <td>
                    {n.recipients.map((r) => (
                      <span key={r} className="prop-chip nt-chip">
                        {r}
                      </span>
                    ))}
                  </td>
                  <td className="nt-msg">
                    <div className="ellipsis">Subject: {n.subject}</div>
                    {n.body && <div className="ellipsis">{n.body}</div>}
                  </td>
                  <td>{fmtDateTime(n.createdAt)}</td>
                  <td>
                    <span className="person-chip">
                      <UserAvatar id={n.author} name={n.author} size={18} /> <span className="ellipsis nt-author">{n.author}</span>
                    </span>
                  </td>
                  <td>{n.automated ? 'Automated' : 'Manual'}</td>
                </tr>
              ))
            )}
          </tbody>
        </table>
      )}
      {draft && (
        <Modal onClose={() => setDraft(null)} label="New notification" width={620} height="auto">
          <form
            className="small-modal"
            onSubmit={(e) => {
              e.preventDefault();
              create.mutate({ recipients: draft.recipients.split(/[,\s]+/).filter(Boolean), subject: draft.subject, body: draft.body }, { onSuccess: () => setDraft(null) });
            }}
          >
            <div className="modal-title-row">
              <h3>New Notification</h3>
              <button type="button" className="icon-plain" onClick={() => setDraft(null)} title="Close">
                <X size={18} />
              </button>
            </div>
            <label className="form-label">
              Recipients
              <input placeholder="oncall@example.com, #incidents, @sre" value={draft.recipients} onChange={(e) => setDraft({ ...draft, recipients: e.target.value })} required />
            </label>
            <label className="form-label">
              Subject
              <input value={draft.subject} onChange={(e) => setDraft({ ...draft, subject: e.target.value })} required maxLength={300} />
            </label>
            <label className="form-label">
              Message
              <textarea rows={4} value={draft.body} onChange={(e) => setDraft({ ...draft, body: e.target.value })} />
            </label>
            <p className="muted">The notification is recorded on the incident. Delivery to email or chat needs a mail or chat integration.</p>
            {create.isError && <p className="form-error">{errorText(create.error)}</p>}
            <div className="small-modal-actions">
              <button type="button" className="btn" onClick={() => setDraft(null)}>
                Cancel
              </button>
              <button className="btn btn-primary" disabled={create.isPending}>
                Send
              </button>
            </div>
          </form>
        </Modal>
      )}
    </section>
  );
}
