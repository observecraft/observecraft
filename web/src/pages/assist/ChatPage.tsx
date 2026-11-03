import { useCallback, useRef, useState, type ReactNode } from 'react';
import { Link, useLocation, useNavigate, useParams } from 'react-router-dom';
import { BarChart3, ChartLine, History, LoaderCircle, Plus, Settings, ShieldAlert, Siren, Sparkles, Trash2, type LucideIcon } from 'lucide-react';
import { ErrorState } from '../../components/ui/AsyncState';
import { timeAgo } from '../../lib/format';
import { useProfile } from '../settings/queries';
import { Composer, Messages } from './Conversation';
import { useAssistStatus, useConversation, useConversations, useDeleteConversation } from './queries';
import { useAssistChat } from './useAssistChat';
import './assist.css';

const SUGGESTIONS: { text: string; label: ReactNode; tag: string; icon: LucideIcon }[] = [
  {
    text: 'Correlate alerts, logs and traces of my services to find the root cause of what is failing',
    label: (
      <>
        Correlate <b>alerts</b>, <b>logs</b> and <b>traces</b> of my services to find the root cause
      </>
    ),
    tag: 'Incidents',
    icon: Siren,
  },
  {
    text: 'Search for error patterns and anomalies in the logs of all my services',
    label: (
      <>
        Search for <b>error patterns</b> and <b>anomalies</b> in the logs of all my services
      </>
    ),
    tag: 'Logs',
    icon: ChartLine,
  },
  {
    text: 'Which services are degraded and which monitors are triggered right now?',
    label: (
      <>
        Surface <b>degraded services</b> and <b>triggered monitors</b> that matter right now
      </>
    ),
    tag: 'Monitors',
    icon: ShieldAlert,
  },
  {
    text: 'Create a monitor that alerts on a high error rate for order-service',
    label: (
      <>
        Create a <b>monitor</b> for a high <b>error rate</b> on one of my services
      </>
    ),
    tag: 'Monitors',
    icon: BarChart3,
  },
];

function greeting(now = new Date()): string {
  const h = now.getHours();
  return h < 12 ? 'Good morning' : h < 18 ? 'Good afternoon' : 'Good evening';
}

/** Assist → Chat: a new conversation (with suggestions) or an earlier one (`/assist/c/:id`). A conversation
 * started here keeps its chat mounted when its URL changes, so the reply goes on streaming. */
export function ChatPage() {
  const { id } = useParams();
  const navigate = useNavigate();
  const location = useLocation();
  const created = useRef(new Set<string>());
  const own = !!id && created.current.has(id);
  const conversation = useConversation(own ? undefined : id);
  const fresh = (location.state as { fresh?: number } | null)?.fresh ?? 0;
  const onCreated = useCallback(
    (cid: string) => {
      created.current.add(cid);
      navigate(`/assist/c/${cid}`, { replace: true, state: location.state });
    },
    [navigate, location.state],
  );
  if (id && !own && conversation.isError) return <ErrorState error={conversation.error} onRetry={() => conversation.refetch()} />;
  if (id && !own && conversation.isPending) return <LoaderCircle className="ai-spin ai-page-loading" size={22} />;
  return <Chat key={id && !own ? id : `new-${fresh}`} initial={id && !own ? conversation.data : undefined} onCreated={onCreated} />;
}

function Chat({ initial, onCreated }: { initial: Parameters<typeof useAssistChat>[0]; onCreated: (id: string) => void }) {
  const navigate = useNavigate();
  const me = useProfile();
  const status = useAssistStatus();
  const [history, setHistory] = useState(false);
  const [autoApprove, setAutoApprove] = useState(false);
  const [draft, setDraft] = useState<{ text: string }>();
  const chat = useAssistChat(initial, 'chat', onCreated);
  const user = { id: me.data?.id ?? 'me', name: me.data?.name ?? 'You' };
  const first = (me.data?.name ?? '').split(/\s+/)[0];
  const empty = chat.messages.length === 0;
  return (
    <div className="ai-page">
      <div className="ai-toolbar">
        <button className="icon-btn" title="New chat" onClick={() => navigate('/assist', { state: { fresh: Date.now() } })}>
          <Plus size={16} />
        </button>
        <button className={`icon-btn${history ? ' active' : ''}`} title="Conversation history" onClick={() => setHistory((h) => !h)}>
          <History size={16} />
        </button>
        <Link className="icon-btn" title="Settings" to="/assist/settings">
          <Settings size={16} />
        </Link>
        <Link className="icon-btn" title="Reports" to="/assist/reports">
          <BarChart3 size={16} />
        </Link>
      </div>
      <div className="ai-body">
        {history && <HistoryList current={chat.id} />}
        <div className={`ai-main${empty ? ' home' : ''}`}>
          {status.data && !status.data.configured && <NotConfigured />}
          {empty ? (
            <div className="ai-home">
              <div className="ai-kicker">
                <Sparkles size={20} /> Assist Chat <span className="ai-new">NEW</span>
              </div>
              <h1>
                {greeting()}
                {first && (
                  <>
                    , <span className="ai-name">{first}</span>
                  </>
                )}
              </h1>
              <Composer chat={chat} autoApprove={autoApprove} onAutoApprove={setAutoApprove} autoFocus draft={draft} />
              <div className="ai-suggestions">
                {SUGGESTIONS.map((s) => (
                  <button key={s.text} className="ai-suggestion" onClick={() => setDraft({ text: s.text })}>
                    <span>{s.label}</span>
                    <span className="ai-suggestion-tag">
                      <s.icon size={13} /> {s.tag}
                    </span>
                  </button>
                ))}
              </div>
            </div>
          ) : (
            <>
              <Messages chat={chat} me={user} autoApprove={autoApprove} />
              <div className="ai-dock">
                <Composer chat={chat} autoApprove={autoApprove} onAutoApprove={setAutoApprove} placeholder="Ask a follow-up" />
              </div>
            </>
          )}
        </div>
      </div>
    </div>
  );
}

function HistoryList({ current }: { current?: string }) {
  const navigate = useNavigate();
  const list = useConversations();
  const remove = useDeleteConversation();
  return (
    <aside className="ai-history" aria-label="Conversation history">
      <h3>Conversations</h3>
      {list.isPending && <p className="muted">Loading…</p>}
      {list.data?.length === 0 && <p className="muted">No conversations yet.</p>}
      {list.data?.map((c) => (
        <div key={c.id} className={`ai-history-item${c.id === current ? ' active' : ''}`}>
          <button className="ai-history-open" onClick={() => navigate(`/assist/c/${c.id}`)} title={c.title}>
            <span className="ellipsis">{c.title}</span>
            <span className="muted">{timeAgo(c.updatedAt)}</span>
          </button>
          <button
            className="icon-plain"
            title="Delete conversation"
            onClick={() =>
              remove.mutate(c.id, {
                onSuccess: () => {
                  if (c.id === current) navigate('/assist', { state: { fresh: Date.now() } });
                },
              })
            }
          >
            <Trash2 size={13} />
          </button>
        </div>
      ))}
    </aside>
  );
}

export function NotConfigured() {
  return (
    <div className="ai-setup">
      <b>Assist needs a model.</b> An administrator sets one for this install: <code>OC_ASSIST_PROVIDER=anthropic</code> with <code>OC_ASSIST_API_KEY</code>, or <code>OC_ASSIST_PROVIDER=openai</code>{' '}
      with <code>OC_ASSIST_BASE_URL</code> and <code>OC_ASSIST_MODEL</code> for any OpenAI-compatible server (vLLM, LiteLLM, Ollama, Azure OpenAI).
    </div>
  );
}
