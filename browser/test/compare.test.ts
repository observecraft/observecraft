import { test } from 'node:test';
import assert from 'node:assert/strict';
import { compare, fill, mask } from '../src/steps.ts';

test('checks compare like Datadog', () => {
  const cases: [Parameters<typeof compare>[0], string, string, boolean][] = [
    ['equals', 'Blue mug', 'Blue mug', true],
    ['equals', 'Blue mug ', 'Blue mug', true],
    ['notEquals', 'a', 'b', true],
    ['contains', 'Results for mugs', 'mugs', true],
    ['notContains', 'Results for mugs', 'cups', true],
    ['startsWith', 'https://shop.example.com/cart', 'https://shop', true],
    ['notStartsWith', 'https://shop.example.com/cart', 'http://', true],
    ['greater', '12.50', '10', true],
    ['lower', '12.50', '10', false],
    ['greaterEquals', '10', '10', true],
    ['lowerEquals', 'abc', '10', false],
    ['matchRegex', '/p/42', '^/p/\\d+$', true],
    ['isEmpty', '  ', '', true],
    ['notIsEmpty', 'x', '', true],
  ];
  for (const [check, actual, expected, ok] of cases) assert.equal(compare(check, actual, expected), ok, `${check} ${actual} ${expected}`);
});

test('variables fill {{ NAME }} and unknown ones are reported', () => {
  const missing: string[] = [];
  assert.equal(fill('https://{{ HOST }}/x?u={{USER}}', { HOST: 'shop.example.com', USER: 'ana' }, missing), 'https://shop.example.com/x?u=ana');
  assert.equal(fill('{{ NOPE }}', {}, missing), '{{ NOPE }}');
  assert.deepEqual(missing, ['NOPE']);
});

test('secrets are masked', () => {
  assert.equal(mask('typed hunter2 into #pw', ['hunter2', '']), 'typed ****** into #pw');
});
