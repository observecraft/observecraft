import { useState } from 'react';
import { Link } from 'react-router-dom';
import { ExternalLink, Plus, Sparkles, X } from 'lucide-react';
import { useProfile } from '../settings/queries';
import { NotConfigured } from './ChatPage';
import { Composer, Messages } from './Conversation';
import { useAssistStatus } from './queries';
import { useAssistChat } from './useAssistChat';
import './assist.css';

/** Ask Assist: a side panel over any page, opened from the top bar. Its conversation stays while the panel is
 * closed, until "New chat". */
export function AskAssistPanel({ open, onClose }: { open: boolean; onClose: () => void }) {
  const [round, setRound] = useState(0);
  const [opened, setOpened] = useState(open);
  if (open && !opened) setOpened(true);
  if (!opened) return null;
  return <Panel key={round} hidden={!open} onClose={onClose} onNew={() => setRound((r) => r + 1)} />;
}

function Panel({ hidden, onClose, onNew }: { hidden: boolean; onClose: () => void; onNew: () => void }) {
  const me = useProfile();
  const status = useAssistStatus();
  const [autoApprove, setAutoApprove] = useState(false);
  const chat = useAssistChat(undefined, 'panel');
  const user = { id: me.data?.id ?? 'me', name: me.data?.name ?? 'You' };
  return (
    <aside className="ai-panel" aria-label="Ask Assist" hidden={hidden}>
      <header className="ai-panel-head">
        <Sparkles size={15} /> <b>Ask Assist</b>
        <span className="ai-panel-actions">
          {chat.id && (
            <Link className="icon-btn" title="Open in Assist Chat" to={`/assist/c/${chat.id}`} onClick={onClose}>
              <ExternalLink size={14} />
            </Link>
          )}
          <button className="icon-btn" title="New chat" disabled={chat.streaming} onClick={onNew}>
            <Plus size={14} />
          </button>
          <button className="icon-btn" title="Close" onClick={onClose}>
            <X size={14} />
          </button>
        </span>
      </header>
      <div className="ai-panel-body">
        {status.data && !status.data.configured && <NotConfigured error={status.data.error} />}
        {chat.messages.length === 0 ? (
          <p className="muted ai-panel-hint">Ask about your logs, metrics, traces, monitors, incidents or Kubernetes. Assist reads them for you and links what it finds.</p>
        ) : (
          <Messages chat={chat} me={user} autoApprove={autoApprove} />
        )}
      </div>
      <div className="ai-dock">
        <Composer chat={chat} autoApprove={autoApprove} onAutoApprove={setAutoApprove} autoFocus placeholder="Ask Assist" />
      </div>
    </aside>
  );
}
