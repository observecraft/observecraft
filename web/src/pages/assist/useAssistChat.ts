import { useCallback, useEffect, useRef, useState } from 'react';
import { useQueryClient } from '@tanstack/react-query';
import { approveActions, sendMessage, type AssistEvent, type AssistSurface, type ConversationView, type ViewMessage } from '../../api/assist/client';
import { applyEvent } from './chatState';
import { assistKeys } from './queries';

export interface AssistChat {
  id: string | undefined;
  messages: ViewMessage[];
  streaming: boolean;
  awaitingApproval: boolean;
  error: string | null;
  send: (text: string, autoApprove: boolean) => void;
  approve: (approved: string[], autoApprove: boolean) => void;
  stop: () => void;
}

export function useAssistChat(initial: ConversationView | undefined, surface: AssistSurface, onCreated?: (id: string) => void): AssistChat {
  const qc = useQueryClient();
  const [id, setId] = useState(initial?.id);
  const [messages, setMessages] = useState<ViewMessage[]>(initial?.messages ?? []);
  const [streaming, setStreaming] = useState(false);
  const [awaitingApproval, setAwaiting] = useState(initial?.awaitingApproval ?? false);
  const [error, setError] = useState<string | null>(null);
  const abort = useRef<AbortController | null>(null);

  useEffect(() => {
    setId(initial?.id);
    setMessages(initial?.messages ?? []);
    setAwaiting(initial?.awaitingApproval ?? false);
    setError(null);
  }, [initial]);
  useEffect(() => () => abort.current?.abort(), []);

  const run = useCallback(
    async (start: (onEvent: (e: AssistEvent) => void, signal: AbortSignal) => Promise<void>) => {
      const controller = new AbortController();
      abort.current = controller;
      setStreaming(true);
      setError(null);
      try {
        await start((e) => {
          if (e.type === 'conversation') {
            setId(e.id);
            onCreated?.(e.id);
          } else if (e.type === 'error') setError(e.message);
          else if (e.type === 'done') setAwaiting(e.awaiting_approval);
          else setMessages((m) => applyEvent(m, e));
        }, controller.signal);
      } catch (err) {
        if (!controller.signal.aborted) setError(err instanceof Error ? err.message : 'Assist failed');
      } finally {
        setStreaming(false);
        qc.invalidateQueries({ queryKey: assistKeys.conversations });
      }
    },
    [onCreated, qc],
  );

  const send = useCallback(
    (text: string, autoApprove: boolean) => {
      const message = text.trim();
      if (!message || streaming) return;
      setMessages((m) => [...m, { role: 'user', text: message, tools: [] }, { role: 'assistant', text: '', tools: [] }]);
      void run((onEvent, signal) => sendMessage({ conversationId: id ?? null, message, surface, autoApprove }, onEvent, signal));
    },
    [id, run, streaming, surface],
  );

  const approve = useCallback(
    (approved: string[], autoApprove: boolean) => {
      if (!id || streaming) return;
      setAwaiting(false);
      void run((onEvent, signal) => approveActions(id, approved, autoApprove, onEvent, signal));
    },
    [id, run, streaming],
  );

  return { id, messages, streaming, awaitingApproval, error, send, approve, stop: () => abort.current?.abort() };
}
