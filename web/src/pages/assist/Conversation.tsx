// A conversation with Assist: the messages (answers as Markdown, the tools each answer used, actions
// waiting for approval) and the composer. Used by Assist Chat and the Ask Assist panel.
import { useEffect, useRef, useState, type KeyboardEvent, type ReactNode } from 'react';
import { Link } from 'react-router-dom';
import ReactMarkdown, { type Components } from 'react-markdown';
import remarkGfm from 'remark-gfm';
import { ArrowUp, Check, ChevronDown, ChevronRight, CircleAlert, CircleCheck, LoaderCircle, Square, Wrench, X } from 'lucide-react';
import type { ViewMessage, ViewTool } from '../../api/assist/client';
import { CodeBlock } from '../../components/code/CodeBlock';
import { UserAvatar } from '../../components/ui/UserAvatar';
import type { AssistChat } from './useAssistChat';

const text = (children: ReactNode): string => (Array.isArray(children) ? children.map(text).join('') : typeof children === 'string' ? children : '');

/** Links inside the app open in place; others in a new tab. Raw HTML is not rendered. */
const MARKDOWN: Components = {
  a: ({ href, children }) =>
    href?.startsWith('/') ? (
      <Link to={href}>{children}</Link>
    ) : href && /^https?:\/\//.test(href) ? (
      <a href={href} target="_blank" rel="noreferrer noopener">
        {children}
      </a>
    ) : (
      <>{children}</>
    ),
  img: () => null,
  pre: ({ children }) => <>{children}</>,
  code: ({ className, children }) => {
    const code = text(children).replace(/\n$/, '');
    return className?.startsWith('language-') || code.includes('\n') ? <CodeBlock code={code} /> : <code>{children}</code>;
  },
  table: ({ children }) => (
    <div className="ai-table">
      <table>{children}</table>
    </div>
  ),
};

/** What a tool did, in words. */
const TOOL_LABEL: Record<string, string> = {
  search_logs: 'Searched logs',
  query_metrics: 'Queried metrics',
  list_services: 'Listed services',
  search_traces: 'Searched traces',
  search_monitors: 'Searched monitors',
  get_monitor: 'Read a monitor',
  create_monitor: 'Create a monitor',
  search_incidents: 'Searched incidents',
  get_incident: 'Read an incident',
  declare_incident: 'Declare an incident',
  search_error_issues: 'Searched Error Tracking',
  search_kubernetes: 'Searched Kubernetes',
};

function ToolRow({ tool }: { tool: ViewTool }) {
  const [open, setOpen] = useState(false);
  const icon =
    tool.status === 'running' ? (
      <LoaderCircle size={13} className="ai-spin" />
    ) : tool.status === 'error' ? (
      <CircleAlert size={13} className="ai-bad" />
    ) : tool.status === 'pending' ? (
      <CircleAlert size={13} className="ai-warn" />
    ) : (
      <CircleCheck size={13} className="ai-ok" />
    );
  return (
    <div className="ai-tool">
      <button className="ai-tool-head" onClick={() => setOpen((o) => !o)} aria-expanded={open}>
        {open ? <ChevronDown size={13} /> : <ChevronRight size={13} />}
        {icon}
        <span>{TOOL_LABEL[tool.name] ?? tool.name}</span>
        {tool.status === 'pending' && <span className="ai-tool-tag">waiting for approval</span>}
        {tool.status === 'error' && tool.summary && <span className="ai-tool-summary ai-bad">{tool.summary}</span>}
      </button>
      {open && (
        <div className="ai-tool-body">
          <pre>{JSON.stringify(tool.input, null, 2)}</pre>
          {tool.summary && <pre className="ai-tool-result">{tool.summary}</pre>}
        </div>
      )}
    </div>
  );
}

function Approval({ tools, chat, autoApprove }: { tools: ViewTool[]; chat: AssistChat; autoApprove: boolean }) {
  return (
    <div className="ai-approval">
      <div className="ai-approval-head">
        <Wrench size={15} /> Assist wants to {tools.length === 1 ? (TOOL_LABEL[tools[0].name] ?? tools[0].name).toLowerCase() : `run ${tools.length} actions`}
      </div>
      {tools.map((t) => (
        <pre key={t.id}>{JSON.stringify(t.input, null, 2)}</pre>
      ))}
      <div className="ai-approval-actions">
        <button
          className="btn btn-primary btn-sm"
          disabled={chat.streaming}
          onClick={() =>
            chat.approve(
              tools.map((t) => t.id),
              autoApprove,
            )
          }
        >
          <Check size={14} /> Approve
        </button>
        <button className="btn btn-sm" disabled={chat.streaming} onClick={() => chat.approve([], autoApprove)}>
          <X size={14} /> Decline
        </button>
      </div>
    </div>
  );
}

function Message({ m, me, last, chat, autoApprove }: { m: ViewMessage; me: { id: string; name: string }; last: boolean; chat: AssistChat; autoApprove: boolean }) {
  if (m.role === 'user') {
    return (
      <div className="ai-msg ai-user">
        <UserAvatar id={me.id} name={me.name} size={24} />
        <div className="ai-user-text">{m.text}</div>
      </div>
    );
  }
  const pending = m.tools.filter((t) => t.status === 'pending');
  const thinking = last && chat.streaming && !m.text && m.tools.every((t) => t.status !== 'running');
  return (
    <div className="ai-msg ai-assistant">
      {m.tools.length > 0 && (
        <div className="ai-tools">
          {m.tools.map((t) => (
            <ToolRow key={t.id} tool={t} />
          ))}
        </div>
      )}
      {m.text && (
        <div className="ai-md">
          <ReactMarkdown remarkPlugins={[remarkGfm]} components={MARKDOWN}>
            {m.text}
          </ReactMarkdown>
        </div>
      )}
      {thinking && (
        <div className="ai-thinking">
          <LoaderCircle size={14} className="ai-spin" /> Thinking…
        </div>
      )}
      {last && chat.awaitingApproval && pending.length > 0 && <Approval tools={pending} chat={chat} autoApprove={autoApprove} />}
    </div>
  );
}

export function Composer({
  chat,
  autoApprove,
  onAutoApprove,
  placeholder = 'Ask, investigate and act across your stack',
  autoFocus,
  draft,
}: {
  chat: AssistChat;
  autoApprove: boolean;
  onAutoApprove: (v: boolean) => void;
  placeholder?: string;
  autoFocus?: boolean;
  /** Text put in the box from outside (a suggestion). */
  draft?: { text: string };
}) {
  const [text, setText] = useState('');
  const box = useRef<HTMLTextAreaElement>(null);
  useEffect(() => {
    if (draft) {
      setText(draft.text);
      box.current?.focus();
    }
  }, [draft]);
  const submit = () => {
    if (!text.trim() || chat.streaming || chat.awaitingApproval) return;
    chat.send(text, autoApprove);
    setText('');
  };
  const onKey = (e: KeyboardEvent<HTMLTextAreaElement>) => {
    if (e.key === 'Enter' && !e.shiftKey && !e.nativeEvent.isComposing) {
      e.preventDefault();
      submit();
    }
  };
  return (
    <div className="ai-composer">
      <textarea
        ref={box}
        autoFocus={autoFocus}
        rows={3}
        value={text}
        placeholder={chat.awaitingApproval ? 'Approve or decline the action above first' : placeholder}
        disabled={chat.awaitingApproval}
        onChange={(e) => setText(e.target.value)}
        onKeyDown={onKey}
        aria-label="Message Assist"
      />
      <div className="ai-composer-bar">
        <label className="ai-auto" title="Run actions that change something (create a monitor, declare an incident) without asking">
          <button type="button" role="switch" aria-checked={autoApprove} className={`switch${autoApprove ? ' on' : ''}`} onClick={() => onAutoApprove(!autoApprove)} />
          Auto-approve
        </label>
        {chat.streaming ? (
          <button className="ai-send" title="Stop" onClick={chat.stop}>
            <Square size={14} />
          </button>
        ) : (
          <button className="ai-send" title="Send" disabled={!text.trim() || chat.awaitingApproval} onClick={submit}>
            <ArrowUp size={16} />
          </button>
        )}
      </div>
    </div>
  );
}

export function Messages({ chat, me, autoApprove }: { chat: AssistChat; me: { id: string; name: string }; autoApprove: boolean }) {
  const end = useRef<HTMLDivElement>(null);
  const lastText = chat.messages[chat.messages.length - 1]?.text.length ?? 0;
  useEffect(() => {
    end.current?.scrollIntoView({ block: 'end' });
  }, [chat.messages.length, lastText, chat.awaitingApproval]);
  return (
    <div className="ai-messages">
      {chat.messages.map((m, i) => (
        <Message key={i} m={m} me={me} last={i === chat.messages.length - 1} chat={chat} autoApprove={autoApprove} />
      ))}
      {chat.error && (
        <div className="ai-error">
          <CircleAlert size={14} /> {chat.error}
        </div>
      )}
      <div ref={end} />
    </div>
  );
}
