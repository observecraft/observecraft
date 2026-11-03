import type { AssistEvent, ViewMessage } from '../../api/assist/types';

/** Applies one streamed event to the messages: the reply is the last (assistant) message. */
export function applyEvent(messages: ViewMessage[], e: AssistEvent): ViewMessage[] {
  const out = messages.slice();
  let last = out[out.length - 1];
  if (!last || last.role !== 'assistant') {
    last = { role: 'assistant', text: '', tools: [] };
    out.push(last);
  } else {
    last = { ...last, tools: last.tools.slice() };
    out[out.length - 1] = last;
  }
  switch (e.type) {
    case 'text':
      last.text += e.text;
      break;
    case 'tool_start': {
      const at = last.tools.findIndex((t) => t.id === e.id);
      const tool = { id: e.id, name: e.name, input: e.input, status: 'running', summary: '' };
      if (at >= 0) last.tools[at] = tool;
      else last.tools.push(tool);
      break;
    }
    case 'tool_end':
      last.tools = last.tools.map((t) => (t.id === e.id ? { ...t, status: e.ok ? 'ok' : 'error', summary: e.summary } : t));
      break;
    case 'approval':
      if (!last.tools.some((t) => t.id === e.id)) last.tools.push({ id: e.id, name: e.name, input: e.input, status: 'pending', summary: '' });
      break;
    default:
      return messages;
  }
  return out;
}

/** One conversation's messages and its streaming reply. `onCreated` gets the id of a new conversation. */
