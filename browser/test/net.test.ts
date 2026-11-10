import { test } from 'node:test';
import assert from 'node:assert/strict';
import { checkAddress, forbiddenReason, pick, resolve } from '../src/net.ts';

// The same cases as the server's synthetics_net.rs: both sides refuse the same addresses.
test('public addresses pass', () => {
  for (const ok of ['93.184.215.14', '8.8.8.8', '1.1.1.1', '172.32.0.1', '100.128.0.1', '2606:4700:4700::1111', '2a00:1450:4001:80b::200e', '64:ff9b::808:808', '2002:808:808::1']) {
    assert.equal(forbiddenReason(ok), null, ok);
  }
});

test('internal IPv4 addresses are refused', () => {
  const cases: [string, string][] = [
    ['127.0.0.1', 'a loopback address'],
    ['127.255.0.9', 'a loopback address'],
    ['10.1.2.3', 'a private address'],
    ['172.16.0.1', 'a private address'],
    ['172.31.255.255', 'a private address'],
    ['192.168.1.10', 'a private address'],
    ['169.254.169.254', 'a link-local address'],
    ['0.0.0.0', 'an unspecified address'],
    ['0.1.2.3', 'an unspecified address'],
    ['100.64.0.1', 'a shared (carrier-grade NAT) address'],
    ['224.0.0.251', 'a multicast address'],
    ['255.255.255.255', 'a reserved address'],
    ['240.0.0.1', 'a reserved address'],
    ['192.0.0.170', 'a reserved address'],
    ['198.18.0.1', 'a reserved (benchmarking) address'],
  ];
  for (const [addr, reason] of cases) assert.equal(forbiddenReason(addr), reason, addr);
});

test('internal IPv6 addresses are refused', () => {
  const cases: [string, string][] = [
    ['::1', 'a loopback address'],
    ['::', 'an unspecified address'],
    ['fd00::1', 'a unique-local address'],
    ['fc12:3456::1', 'a unique-local address'],
    ['fe80::1', 'a link-local address'],
    ['fec0::1', 'a site-local address'],
    ['ff02::1', 'a multicast address'],
    ['100::1', 'a discard-only address'],
    ['::ffff:127.0.0.1', 'a loopback address'],
    ['::ffff:169.254.169.254', 'a link-local address'],
    ['::ffff:10.0.0.1', 'a private address'],
    ['::10.0.0.1', 'a private address'],
    ['::8.8.8.8', 'a reserved address'],
    ['64:ff9b::a00:1', 'a private address'],
    ['2002:a9fe:a9fe::1', 'a link-local address'],
    ['2002:7f00:1::1', 'a loopback address'],
  ];
  for (const [addr, reason] of cases) assert.equal(forbiddenReason(addr), reason, addr);
});

test('the operator setting allows private targets', () => {
  assert.match(checkAddress('10.0.0.5', false) ?? '', /10\.0\.0\.5 is a private address.*OC_SYNTHETICS_ALLOW_PRIVATE/);
  assert.equal(checkAddress('10.0.0.5', true), null);
  assert.equal(checkAddress('8.8.4.4', false), null);
});

test('a host with any internal address is refused', () => {
  assert.equal(pick(['2606:4700::1', '93.184.215.14'], false), '93.184.215.14', 'IPv4 first');
  assert.throws(() => pick(['93.184.215.14', '10.0.0.7'], false), /10\.0\.0\.7/);
  assert.equal(pick(['93.184.215.14', '10.0.0.7'], true), '93.184.215.14');
  assert.throws(() => pick([], true), /no address/);
});

test('literals are checked without a lookup', async () => {
  await assert.rejects(resolve('127.0.0.1', false), /loopback/);
  await assert.rejects(resolve('[::1]', false), /loopback/);
  assert.equal(await resolve('[::1]', true), '::1');
  await assert.rejects(resolve('localhost', false), /loopback/);
});
