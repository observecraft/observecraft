// Browser test steps (Datadog's browser step model): labels, defaults and the checks the editor shows before the
// server's (server/crates/oc-api/src/browser/steps.rs) refuse a step.
import type { BrowserElement, BrowserStep, BrowserStepCheck, BrowserStepType } from './types';

export const STEP_GROUPS: { title: string; types: BrowserStepType[] }[] = [
  { title: 'Actions', types: ['click', 'typeText', 'selectOption', 'hover', 'pressKey', 'scroll', 'goToUrl', 'refresh', 'wait'] },
  { title: 'Assertions', types: ['assertElementPresent', 'assertElementContent', 'assertElementAttribute', 'assertPageContains', 'assertPageLacks', 'assertCurrentUrl', 'assertFromJavascript'] },
  { title: 'Variables', types: ['extractVariable', 'extractFromJavascript'] },
];

export const STEP_LABEL: Record<BrowserStepType, string> = {
  goToUrl: 'Navigate to URL',
  click: 'Click',
  typeText: 'Type text',
  selectOption: 'Select option',
  hover: 'Hover',
  pressKey: 'Press key',
  scroll: 'Scroll',
  refresh: 'Refresh',
  wait: 'Wait',
  assertElementPresent: 'Element is present',
  assertElementContent: 'Element content',
  assertElementAttribute: 'Element attribute',
  assertPageContains: 'Page contains text',
  assertPageLacks: 'Page lacks text',
  assertCurrentUrl: 'Current URL',
  assertFromJavascript: 'JavaScript assertion',
  extractVariable: 'Extract from element',
  extractFromJavascript: 'Extract from JavaScript',
};

const ELEMENT_STEPS: BrowserStepType[] = ['click', 'typeText', 'selectOption', 'hover', 'assertElementPresent', 'assertElementContent', 'assertElementAttribute', 'extractVariable'];
export const needsElement = (t: BrowserStepType) => ELEMENT_STEPS.includes(t);
export const isAssertion = (t: BrowserStepType) => t.startsWith('assert');
export const usesCheck = (t: BrowserStepType) => t === 'assertElementContent' || t === 'assertElementAttribute' || t === 'assertCurrentUrl';

export const CHECK_LABEL: Record<BrowserStepCheck, string> = {
  equals: 'equals',
  notEquals: 'does not equal',
  contains: 'contains',
  notContains: 'does not contain',
  startsWith: 'starts with',
  notStartsWith: 'does not start with',
  greater: 'is greater than',
  lower: 'is lower than',
  greaterEquals: 'is greater than or equal to',
  lowerEquals: 'is lower than or equal to',
  matchRegex: 'matches the regex',
  isEmpty: 'is empty',
  notIsEmpty: 'is not empty',
};
export const CHECKS = Object.keys(CHECK_LABEL) as BrowserStepCheck[];

export const KEYS = ['Enter', 'Tab', 'Escape', 'Backspace', 'Delete', 'ArrowUp', 'ArrowDown', 'ArrowLeft', 'ArrowRight', 'Home', 'End', 'PageUp', 'PageDown', 'Space', 'Insert', 'F5'];
export const MODIFIERS = ['Alt', 'Control', 'Meta', 'Shift'];

export const DEVICES: { id: string; label: string; size: string }[] = [
  { id: 'chrome.laptop_large', label: 'Laptop Large', size: '1440 × 1100' },
  { id: 'chrome.tablet', label: 'Tablet', size: '768 × 1020' },
  { id: 'chrome.mobile_small', label: 'Mobile Small', size: '320 × 550' },
];

/** Browser tests run every 5 minutes at most. */
export const MIN_BROWSER_FREQUENCY = 300;

/** Datadog's failure codes, in words. */
export const FAILURE_LABEL: Record<string, string> = {
  LOCATE_ELEMENT: 'Element not found',
  ASSERTION_FAILURE: 'Assertion failed',
  ELEMENT_NOT_INTERACTABLE: 'Element not interactable',
  OPEN_URL: 'Page could not be opened',
  STEP_TIMEOUT: 'Step timed out',
  TEST_TIMEOUT: 'Test timed out',
  EVALUATE_JAVASCRIPT: 'JavaScript failed',
  EXTRACT_VARIABLE: 'Variable not extracted',
  SELECT_OPTION: 'Option not selected',
  PRESS_KEY: 'Key not pressed',
  UNKNOWN_VARIABLE: 'Unknown variable',
  INTERNAL_ERROR: 'Internal error',
};

export function cssElement(selector: string): BrowserElement {
  return { userLocator: { failTestOnCannotLocate: true, values: [{ type: 'css', value: selector }] } };
}

/** `params.value` as text (wait seconds come back as numbers). */
export function stepValue(s: BrowserStep): string {
  const v = s.params?.value as unknown;
  return v == null ? '' : String(v);
}

/** A new step of a type with sensible parameters. */
export function newStep(type: BrowserStepType): BrowserStep {
  const params: BrowserStep['params'] = {};
  if (needsElement(type)) params.element = cssElement('');
  if (usesCheck(type)) params.check = type === 'assertCurrentUrl' ? 'contains' : 'equals';
  if (type === 'wait') params.value = '1';
  if (type === 'pressKey') params.value = 'Enter';
  if (type === 'goToUrl') params.value = 'https://';
  if (type === 'scroll') Object.assign(params, { x: 0, y: 500 });
  if (type === 'assertFromJavascript') params.code = 'return document.title.length > 0;';
  if (type === 'extractFromJavascript') params.code = 'return document.title;';
  if (type === 'extractVariable' || type === 'extractFromJavascript') params.variable = { name: '', secure: false };
  return { name: '', type, params };
}

function elementLabel(s: BrowserStep): string {
  const el = s.params?.element;
  return el?.description || el?.userLocator?.values?.[0]?.value || 'element';
}

/** The step's name, or the name the server gives a step without one. */
export function stepName(s: BrowserStep): string {
  if (s.name?.trim()) return s.name;
  const v = stepValue(s);
  const el = elementLabel(s);
  switch (s.type) {
    case 'goToUrl':
      return `Navigate to ${v}`;
    case 'click':
      return `Click on ${el}`;
    case 'typeText':
      return `Type text on ${el}`;
    case 'selectOption':
      return `Select option '${v}' in ${el}`;
    case 'hover':
      return `Hover over ${el}`;
    case 'pressKey':
      return `Press ${v}`;
    case 'scroll':
      return 'Scroll';
    case 'refresh':
      return 'Refresh the page';
    case 'wait':
      return `Wait ${v} seconds`;
    case 'assertElementPresent':
      return `Test ${el} is present`;
    case 'assertElementContent':
      return `Test ${el} content`;
    case 'assertElementAttribute':
      return `Test ${el} attribute ${s.params?.attribute ?? ''}`;
    case 'assertPageContains':
      return `Test page contains '${v}'`;
    case 'assertPageLacks':
      return `Test page lacks '${v}'`;
    case 'assertCurrentUrl':
      return `Test current URL ${v}`;
    case 'assertFromJavascript':
      return 'Test custom JavaScript assertion';
    case 'extractVariable':
    case 'extractFromJavascript':
      return `Extract variable ${s.params?.variable?.name ?? ''}`;
  }
}

const VARIABLE = /^[A-Z_][A-Z0-9_]{0,79}$/;

/** Why the server would refuse the step, or null. */
export function stepProblem(s: BrowserStep): string | null {
  const p = s.params ?? {};
  const v = stepValue(s);
  if (s.timeout != null && (s.timeout < 1 || s.timeout > 300)) return 'The timeout is 1–300 seconds';
  if (needsElement(s.type) && !p.element?.userLocator?.values?.some((l) => l.value.trim())) return 'Choose the element it acts on (a CSS or XPath locator)';
  switch (s.type) {
    case 'goToUrl':
      return /^https?:\/\/[^/\s]+/.test(v.trim()) ? null : 'Enter an http or https URL';
    case 'typeText':
      return v ? null : 'Enter the text to type';
    case 'selectOption':
      return v.trim() ? null : 'Name the option to select (its value or label)';
    case 'pressKey':
      return KEYS.includes(v) || [...v].length === 1 ? null : `Press one character or one of ${KEYS.join(', ')}`;
    case 'wait': {
      const n = Number(v);
      return Number.isInteger(n) && n >= 1 && n <= 300 ? null : 'Wait 1–300 seconds';
    }
    case 'assertPageContains':
    case 'assertPageLacks':
      return v.trim() ? null : 'Enter the text to look for';
    case 'assertElementAttribute':
      if (!p.attribute?.trim()) return 'Name the attribute to check';
    // falls through
    case 'assertElementContent':
    case 'assertCurrentUrl':
      if (!p.check) return 'Choose a check';
      if (p.check === 'isEmpty' || p.check === 'notIsEmpty') return null;
      if (!v.trim()) return 'Enter the expected value';
      if (p.check === 'matchRegex' && !/\{\{/.test(v)) {
        try {
          new RegExp(v);
        } catch (e) {
          return `Invalid regular expression: ${(e as Error).message}`;
        }
      }
      return null;
    case 'assertFromJavascript':
      return p.code?.trim() ? null : 'Enter the JavaScript';
    case 'extractFromJavascript':
      if (!p.code?.trim()) return 'Enter the JavaScript';
    // falls through
    case 'extractVariable':
      return VARIABLE.test(p.variable?.name ?? '') ? null : 'Name the variable in capitals, digits and underscores (ORDER_ID)';
    default:
      return null;
  }
}
