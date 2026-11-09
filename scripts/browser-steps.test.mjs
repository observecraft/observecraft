// The browser test editor's step checks (web/src/api/synthetics/browserSteps.ts) refuse what the server's
// browser/steps.rs refuses, with the same cases.
import assert from 'node:assert/strict';
import test from 'node:test';
import { cssElement, newStep, stepName, stepProblem } from '../web/src/api/synthetics/browserSteps.ts';

const step = (type, params = {}, extra = {}) => ({ name: '', type, params, ...extra });

test('valid steps pass and get the server default names', () => {
  const ok = [
    step('goToUrl', { value: 'https://shop.example.com/cart' }),
    step('typeText', { element: cssElement('#email'), value: '{{ EMAIL }}' }),
    step('assertCurrentUrl', { check: 'contains', value: '/checkout' }),
    step('assertPageLacks', { value: 'Error' }),
    step('pressKey', { value: 'Enter', modifiers: ['Shift'] }),
    step('pressKey', { value: 'a' }),
    step('wait', { value: 2 }),
    step('refresh'),
    step('scroll', { x: 0, y: 400 }),
    step('extractVariable', { element: cssElement('.order-id'), variable: { name: 'ORDER_ID' } }),
    step('assertElementContent', { element: cssElement('h1'), check: 'isEmpty' }),
    step('assertElementAttribute', { element: cssElement('a'), attribute: 'href', check: 'matchRegex', value: '^/p/\\d+$' }),
    step('assertFromJavascript', { code: 'return document.title.length > 0' }),
  ];
  for (const s of ok) assert.equal(stepProblem(s), null, JSON.stringify(s));
  assert.equal(stepName(ok[0]), 'Navigate to https://shop.example.com/cart');
  assert.equal(stepName(ok[1]), 'Type text on #email');
  assert.equal(stepName({ ...ok[1], name: 'Fill email' }), 'Fill email');
});

test('invalid steps are refused', () => {
  const bad = [
    step('click'),
    step('click', { element: cssElement('  ') }),
    step('goToUrl', { value: 'shop.example.com' }),
    step('typeText', { element: cssElement('#q') }),
    step('wait', { value: 0 }),
    step('wait', { value: 301 }),
    step('wait', { value: 'soon' }),
    step('pressKey', { value: 'Hyper' }),
    step('assertCurrentUrl', { value: '/x' }),
    step('assertCurrentUrl', { check: 'contains' }),
    step('assertElementContent', { element: cssElement('h1'), check: 'matchRegex', value: '(' }),
    step('assertElementAttribute', { element: cssElement('a'), check: 'equals', value: 'x' }),
    step('assertPageContains', { value: '' }),
    step('extractVariable', { element: cssElement('b'), variable: { name: 'order id' } }),
    step('assertFromJavascript', { code: ' ' }),
    step('selectOption', { element: cssElement('select') }),
    step('refresh', {}, { timeout: 0 }),
    step('refresh', {}, { timeout: 301 }),
  ];
  for (const s of bad) assert.ok(stepProblem(s), JSON.stringify(s));
});

test('new steps start with what their type needs', () => {
  assert.ok(newStep('click').params.element);
  assert.equal(newStep('assertCurrentUrl').params.check, 'contains');
  assert.equal(newStep('wait').params.value, '1');
  assert.ok(stepProblem(newStep('click')), 'an element is still to choose');
  assert.equal(stepProblem(newStep('refresh')), null);
});
