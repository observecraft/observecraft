// The Datadog Agent's conf.d/<check>.d/conf.yaml from an integrations-core configuration spec
// (agentSpecs.generated.json) and the values filled in the form. The form covers one instance's
// options; init_config keeps the check's defaults.

export interface SpecValue {
  type?: 'string' | 'integer' | 'number' | 'boolean' | 'array' | 'object';
  example?: unknown;
  display_default?: unknown;
  items?: { type?: string };
  enum?: unknown[];
  anyOf?: SpecValue[];
}

export interface SpecOption {
  name: string;
  description?: string;
  required?: boolean;
  secret?: boolean;
  multiple?: boolean;
  value?: SpecValue;
  options?: SpecOption[];
}

export interface SpecFile {
  name: string;
  options: SpecOption[];
}

/** Options the form edits: the instance's scalar and string-list options, required ones first. */
export function instanceOptions(file: SpecFile): SpecOption[] {
  const instances = file.options.find((o) => o.name === 'instances')?.options ?? [];
  const editable = instances.filter((o) => formValue(o.value));
  return [...editable.filter((o) => o.required), ...editable.filter((o) => !o.required)];
}

const isScalar = (v?: SpecValue) => !!v && (v.type === 'string' || v.type === 'integer' || v.type === 'number' || v.type === 'boolean');
const isStringList = (v?: SpecValue) => !!v && v.type === 'array' && (v.items?.type ?? 'string') === 'string';

/** The value form the form edits: the value itself, or the first scalar / string-list alternative of `anyOf`. */
export function formValue(v?: SpecValue): SpecValue | undefined {
  if (isScalar(v) || isStringList(v)) return v;
  return v?.anyOf?.find((alt) => isScalar(alt) || isStringList(alt));
}

// Plain scalars the Agent's YAML 1.1 parser would read as something else, or that YAML cannot hold unquoted.
const YAML11_SPECIAL = /^(y|yes|n|no|true|false|on|off|null|~|[-+]?(\d[\d_]*)?\.?\d+([eE][-+]?\d+)?|0x[0-9a-f]+|\.inf|\.nan)$/i;
const NEEDS_QUOTES = /^[\s\-?:,[\]{}#&*!|>'"%@`]|\s$|: | #|^$/;

function scalar(text: string): string {
  return YAML11_SPECIAL.test(text) || NEEDS_QUOTES.test(text) ? `'${text.replace(/'/g, "''")}'` : text;
}

function typed(option: SpecOption, raw: string): string | string[] | null {
  const text = raw.trim();
  if (!text) return null;
  switch (formValue(option.value)?.type) {
    case 'integer':
    case 'number':
      return Number.isFinite(Number(text)) ? text : scalar(text);
    case 'boolean':
      return text === 'true' || text === 'false' ? text : null;
    case 'array': {
      const items = text
        .split(/[,\n]/)
        .map((s) => s.trim())
        .filter(Boolean);
      return items.length ? items.map(scalar) : null;
    }
    default:
      return scalar(text);
  }
}

/** conf.yaml with one instance holding the filled options, in the spec's order. */
export function renderAgentConfig(file: SpecFile, values: Record<string, string>): string {
  const lines = ['init_config: {}', '', 'instances:'];
  let first = true;
  for (const option of file.options.find((o) => o.name === 'instances')?.options ?? []) {
    const value = values[option.name] === undefined ? null : typed(option, values[option.name]);
    if (value === null) continue;
    const prefix = first ? '  - ' : '    ';
    first = false;
    if (Array.isArray(value)) {
      lines.push(`${prefix}${option.name}:`);
      for (const item of value) lines.push(`      - ${item}`);
    } else {
      lines.push(`${prefix}${option.name}: ${value}`);
    }
  }
  if (first) lines.push('  - {}');
  return lines.join('\n') + '\n';
}
