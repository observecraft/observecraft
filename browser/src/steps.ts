// Browser test steps in Datadog's browser step model, as the server validates them (server/crates/oc-api/src/browser/steps.rs),
// and the value helpers the runner uses: comparisons (`check`), `{{ VARIABLES }}` and masking of secure values.

export type StepType =
  | 'goToUrl'
  | 'click'
  | 'typeText'
  | 'selectOption'
  | 'hover'
  | 'pressKey'
  | 'scroll'
  | 'refresh'
  | 'wait'
  | 'assertElementPresent'
  | 'assertElementContent'
  | 'assertElementAttribute'
  | 'assertPageContains'
  | 'assertPageLacks'
  | 'assertCurrentUrl'
  | 'assertFromJavascript'
  | 'extractVariable'
  | 'extractFromJavascript';

export type Check =
  'equals' | 'notEquals' | 'contains' | 'notContains' | 'startsWith' | 'notStartsWith' | 'greater' | 'lower' | 'greaterEquals' | 'lowerEquals' | 'matchRegex' | 'isEmpty' | 'notIsEmpty';

export interface Locator {
  type: 'css' | 'xpath';
  value: string;
}

export interface Element {
  userLocator: { failTestOnCannotLocate?: boolean; values: Locator[] };
  description?: string;
}

export interface StepParams {
  element?: Element;
  value?: string | number;
  check?: Check;
  attribute?: string;
  clickType?: 'primary' | 'double' | 'contextual';
  appendToContent?: boolean;
  modifiers?: string[];
  x?: number;
  y?: number;
  code?: string;
  variable?: { name: string; secure?: boolean };
}

export interface Step {
  name: string;
  type: StepType;
  params?: StepParams;
  allowFailure?: boolean;
  isCritical?: boolean;
  noScreenshot?: boolean;
  /** Seconds to wait for the element or assertion. */
  timeout?: number;
}

export const DEFAULT_STEP_TIMEOUT = 60;

/** Whether `actual` passes `check` against `expected` (numbers for greater/lower, a regular expression for matchRegex). */
export function compare(check: Check, actual: string, expected: string): boolean {
  const a = actual.trim();
  const e = expected.trim();
  const numbers = (test: (x: number, y: number) => boolean) => {
    const x = Number.parseFloat(a);
    const y = Number.parseFloat(e);
    return !Number.isNaN(x) && !Number.isNaN(y) && test(x, y);
  };
  switch (check) {
    case 'equals':
      return a === e;
    case 'notEquals':
      return a !== e;
    case 'contains':
      return a.includes(e);
    case 'notContains':
      return !a.includes(e);
    case 'startsWith':
      return a.startsWith(e);
    case 'notStartsWith':
      return !a.startsWith(e);
    case 'greater':
      return numbers((x, y) => x > y);
    case 'lower':
      return numbers((x, y) => x < y);
    case 'greaterEquals':
      return numbers((x, y) => x >= y);
    case 'lowerEquals':
      return numbers((x, y) => x <= y);
    case 'matchRegex':
      return new RegExp(expected).test(actual);
    case 'isEmpty':
      return a === '';
    case 'notIsEmpty':
      return a !== '';
  }
}

const PLACEHOLDER = /\{\{\s*([A-Za-z0-9_.-]+)\s*\}\}/g;

/** The text with every known `{{ NAME }}` replaced; unknown names are added to `missing`. */
export function fill(text: string, variables: Record<string, string>, missing: string[]): string {
  return text.replace(PLACEHOLDER, (whole, name: string) => {
    if (name in variables) return variables[name];
    if (!missing.includes(name)) missing.push(name);
    return whole;
  });
}

/** The text with every secure value hidden. */
export function mask(text: string, secrets: string[]): string {
  let out = text;
  for (const s of secrets) if (s) out = out.split(s).join('******');
  return out;
}
