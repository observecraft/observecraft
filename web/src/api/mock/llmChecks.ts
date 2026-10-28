// The mock's copy of the server's built-in LLM evaluations (server/crates/oc-api/src/routes/
// llm_checks.rs) and list-price table (llm_attrs.rs), so `make web-mock` shows the same results.
import type { components } from '../generated/schema';

type CheckOutcome = components['schemas']['CheckOutcome'];

export const CHECKS: { id: string; name: string; description: string; heuristic: boolean }[] = [
  { id: 'failure_to_answer', name: 'Failure to answer', description: 'The model returned no text, or a refusal / "cannot help" phrase.', heuristic: true },
  { id: 'error', name: 'Error', description: 'The call failed (span status error).', heuristic: false },
  { id: 'token_outlier', name: 'Token outlier', description: 'Total tokens far above other calls to the same model (above Q3 + 3 × IQR, at least 20 calls).', heuristic: false },
  { id: 'latency_outlier', name: 'Latency outlier', description: 'Duration far above other calls to the same model (above Q3 + 3 × IQR, at least 20 calls).', heuristic: false },
  { id: 'prompt_injection', name: 'Prompt injection', description: 'The user input contains a known injection phrase ("ignore previous instructions", …).', heuristic: true },
  { id: 'pii_in_output', name: 'PII in output', description: 'The output contains an email address, a card-like number (Luhn-valid) or an SSN-like number.', heuristic: true },
];

const REFUSALS = [
  "i'm sorry, but i can't",
  "i'm sorry, i can't",
  'i am sorry, but i cannot',
  "i can't help with",
  'i cannot help with',
  "i can't assist with",
  'i cannot assist with',
  "i'm unable to help",
  'i am unable to help',
  "i'm not able to help",
  "i don't have access to that information",
  'as an ai language model',
];

const INJECTIONS = [
  'ignore previous instructions',
  'ignore all previous instructions',
  'ignore the previous instructions',
  'ignore prior instructions',
  'ignore the above',
  'disregard previous instructions',
  'disregard the system prompt',
  'forget your instructions',
  'reveal your system prompt',
  'print your system prompt',
  'show me your system prompt',
  'you are now dan',
  'developer mode enabled',
  'jailbreak',
];

export interface CallSample {
  model: string;
  durationMs: number;
  error: boolean;
  input: string;
  output: string;
  totalTokens: number;
}

const normalized = (s: string) => s.toLowerCase().replace(/[‘’]/g, "'").split(/\s+/).filter(Boolean).join(' ');

const luhn = (digits: number[]) =>
  digits
    .slice()
    .reverse()
    .reduce((sum, d, i) => sum + (i % 2 === 1 ? (d * 2 > 9 ? d * 2 - 9 : d * 2) : d), 0) %
    10 ===
  0;

/** Kinds of personal data in the text: email address, card number (13–19 digits, Luhn), SSN-like number. */
export function piiKinds(text: string): string[] {
  const found: string[] = [];
  if (/(^|[\s<>()[\]{},;:"'])[A-Za-z0-9._%+-]+@[A-Za-z0-9-]+(\.[A-Za-z0-9-]+)*\.[A-Za-z]{2,}(?=$|[\s<>()[\]{},;:"'.])/.test(text)) found.push('email address');
  for (const m of text.matchAll(/(?<![A-Za-z0-9])\d(?:[ -]?\d){12,18}(?![0-9])/g)) {
    const digits = [...m[0].replace(/\D/g, '')].map(Number);
    if (digits.length >= 13 && digits.length <= 19 && luhn(digits)) {
      found.push('card number');
      break;
    }
  }
  if (/(?<![A-Za-z0-9])\d{3}-\d{2}-\d{4}(?![A-Za-z0-9])/.test(text)) found.push('SSN-like number');
  return found;
}

function quartiles(sorted: number[]): [number, number] {
  const q = (p: number) => {
    const pos = p * (sorted.length - 1);
    const lo = Math.floor(pos);
    const hi = Math.ceil(pos);
    return sorted[lo] + (sorted[hi] - sorted[lo]) * (pos - lo);
  };
  return [q(0.25), q(0.75)];
}

/** Upper outlier fence (Q3 + 3 × IQR) per model, for models with at least 20 positive values. */
export function fences(samples: CallSample[], value: (s: CallSample) => number): Map<string, number> {
  const by = new Map<string, number[]>();
  for (const s of samples) {
    const v = value(s);
    if (v > 0 && !s.error) by.set(s.model, [...(by.get(s.model) ?? []), v]);
  }
  const out = new Map<string, number>();
  for (const [m, vals] of by) {
    if (vals.length < 20) continue;
    const [q1, q3] = quartiles(vals.sort((a, b) => a - b));
    out.set(m, q3 + 3 * (q3 - q1));
  }
  return out;
}

function outlier(check: string, value: number, fence: number | undefined, unit: string): CheckOutcome | null {
  if (fence === undefined || value <= 0) return null;
  return { check, passed: value <= fence, detail: value > fence ? `${Math.round(value)} ${unit}, above ${Math.round(fence)} ${unit} for this model` : '' };
}

/** The checks that apply to one call (fences from its population; empty maps skip the outlier checks). */
export function evaluate(s: CallSample, tokenFences = new Map<string, number>(), latencyFences = new Map<string, number>()): CheckOutcome[] {
  const out: CheckOutcome[] = [{ check: 'error', passed: !s.error, detail: '' }];
  if (!s.error) {
    const answer = normalized(s.output);
    const refusal = REFUSALS.find((p) => answer.includes(p));
    out.push({ check: 'failure_to_answer', passed: !!answer && !refusal, detail: !answer ? 'empty output' : refusal ? `refusal phrase "${refusal}"` : '' });
    const pii = piiKinds(s.output);
    out.push({ check: 'pii_in_output', passed: pii.length === 0, detail: pii.join(', ') });
    for (const o of [outlier('token_outlier', s.totalTokens, tokenFences.get(s.model), 'tokens'), outlier('latency_outlier', s.durationMs, latencyFences.get(s.model), 'ms')]) if (o) out.push(o);
  }
  if (s.input.trim()) {
    const input = normalized(s.input);
    const hit = INJECTIONS.find((p) => input.includes(p));
    out.push({ check: 'prompt_injection', passed: !hit, detail: hit ? `input contains "${hit}"` : '' });
  }
  return out;
}

/** List prices in USD per million (input, output) tokens: a rough estimate, longest prefix wins. */
export const PRICES: [string, number, number][] = [
  ['gpt-4o-mini', 0.15, 0.6],
  ['gpt-4o', 2.5, 10],
  ['gpt-4.1-nano', 0.1, 0.4],
  ['gpt-4.1-mini', 0.4, 1.6],
  ['gpt-4.1', 2, 8],
  ['gpt-3.5-turbo', 0.5, 1.5],
  ['o4-mini', 1.1, 4.4],
  ['o3-mini', 1.1, 4.4],
  ['o3', 2, 8],
  ['text-embedding-3-small', 0.02, 0],
  ['text-embedding-3-large', 0.13, 0],
  ['claude-fable-5', 10, 50],
  ['claude-opus-5-5', 4, 20],
  ['claude-opus-5', 5, 25],
  ['claude-opus-4', 5, 25],
  ['claude-sonnet-5', 2, 10],
  ['claude-sonnet-4', 3, 15],
  ['claude-haiku-4', 1, 5],
  ['gemini-2.0-flash', 0.1, 0.4],
  ['gemini-1.5-flash', 0.075, 0.3],
  ['gemini-1.5-pro', 1.25, 5],
  ['mistral-large', 2, 6],
  ['mistral-small', 0.2, 0.6],
];

export function estimateCost(model: string, inputTokens: number, outputTokens: number): number | null {
  let m = model.toLowerCase();
  m = m.split(/[/:]/).pop() ?? m;
  m = m.replace(/^(us\.)?anthropic\./, '');
  const hit = PRICES.filter(([p]) => m.startsWith(p)).sort((a, b) => b[0].length - a[0].length)[0];
  return hit ? (inputTokens * hit[1] + outputTokens * hit[2]) / 1e6 : null;
}
