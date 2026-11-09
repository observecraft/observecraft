// Where the browser may connect: the same rules as the server's synthetics_net.rs. Every address a host
// resolves to must be public (loopback, private, link-local, cloud metadata, unique-local, multicast and other
// unroutable ranges are refused) unless the operator allows private targets (OC_SYNTHETICS_ALLOW_PRIVATE).
import { lookup } from 'node:dns/promises';
import { isIP } from 'node:net';

function v4Octets(ip: string): number[] {
  return ip.split('.').map(Number);
}

function forbiddenV4([a, b, c, d]: number[]): string | null {
  if (a === 0) return 'an unspecified address';
  if (a === 127) return 'a loopback address';
  if (a === 10 || (a === 172 && (b & 0xf0) === 16) || (a === 192 && b === 168)) return 'a private address';
  if (a === 169 && b === 254) return 'a link-local address';
  if (a === 100 && (b & 0xc0) === 64) return 'a shared (carrier-grade NAT) address';
  if (a >= 224 && a <= 239) return 'a multicast address';
  if ((a === 255 && b === 255 && c === 255 && d === 255) || a >= 240 || (a === 192 && b === 0 && c === 0)) return 'a reserved address';
  if (a === 198 && (b & 0xfe) === 18) return 'a reserved (benchmarking) address';
  return null;
}

/** The eight 16-bit groups of an IPv6 address (an embedded IPv4 tail included). */
function v6Segments(ip: string): number[] {
  let text = ip;
  const tail = /(\d+\.\d+\.\d+\.\d+)$/.exec(text);
  if (tail) {
    const [a, b, c, d] = v4Octets(tail[1]);
    text = `${text.slice(0, tail.index)}${((a << 8) | b).toString(16)}:${((c << 8) | d).toString(16)}`;
  }
  const [head, rest] = text.split('::');
  const part = (s: string | undefined) => (s ? s.split(':').map((x) => parseInt(x, 16)) : []);
  const left = part(head);
  const right = part(rest);
  const zeros = rest === undefined ? [] : new Array(8 - left.length - right.length).fill(0);
  return [...left, ...zeros, ...right];
}

function forbiddenV6(ip: string): string | null {
  const s = v6Segments(ip);
  const embedded = (hi: number, lo: number) => [hi >> 8, hi & 0xff, lo >> 8, lo & 0xff];
  const zero = (from: number, to: number) => s.slice(from, to).every((x) => x === 0);
  // Addresses that carry an IPv4 address are judged by it: mapped, compatible, NAT64 and 6to4.
  if (zero(0, 5) && s[5] === 0xffff) return forbiddenV4(embedded(s[6], s[7]));
  if (zero(0, 8)) return 'an unspecified address';
  if (zero(0, 7) && s[7] === 1) return 'a loopback address';
  if (zero(0, 6)) return forbiddenV4(embedded(s[6], s[7])) ?? 'a reserved address';
  if (s[0] === 0x64 && s[1] === 0xff9b && zero(2, 6)) return forbiddenV4(embedded(s[6], s[7]));
  if (s[0] === 0x2002) return forbiddenV4(embedded(s[1], s[2]));
  if ((s[0] & 0xfe00) === 0xfc00) return 'a unique-local address';
  if ((s[0] & 0xffc0) === 0xfe80) return 'a link-local address';
  if ((s[0] & 0xffc0) === 0xfec0) return 'a site-local address';
  if ((s[0] & 0xff00) === 0xff00) return 'a multicast address';
  if (s[0] === 0x0100 && zero(1, 4)) return 'a discard-only address';
  return null;
}

/** Why `ip` is not a public address, or null when it is. */
export function forbiddenReason(ip: string): string | null {
  return isIP(ip) === 4 ? forbiddenV4(v4Octets(ip)) : forbiddenV6(ip);
}

/** The refusal for one address, or null when the policy allows it. */
export function checkAddress(ip: string, allowPrivate: boolean): string | null {
  const reason = forbiddenReason(ip);
  return reason && !allowPrivate ? `${ip} is ${reason}; private targets are disabled on this server (OC_SYNTHETICS_ALLOW_PRIVATE)` : null;
}

/** The address to connect to: every one of the host's addresses must pass, IPv4 preferred. */
export function pick(addresses: string[], allowPrivate: boolean): string {
  if (addresses.length === 0) throw new Error('The host has no address');
  for (const a of addresses) {
    const refused = checkAddress(a, allowPrivate);
    if (refused) throw new Error(refused);
  }
  return addresses.find((a) => isIP(a) === 4) ?? addresses[0];
}

/** Resolves a host name or IP literal (IPv6 with or without brackets) to the checked address to connect to. */
export async function resolve(host: string, allowPrivate: boolean): Promise<string> {
  const bare = host.replace(/^\[/, '').replace(/\]$/, '');
  if (isIP(bare)) return pick([bare], allowPrivate);
  const found = await lookup(bare, { all: true }).catch((e: Error) => {
    throw new Error(`DNS lookup of ${bare} failed: ${e.message}`);
  });
  try {
    return pick(
      found.map((f) => f.address),
      allowPrivate,
    );
  } catch (e) {
    throw new Error(`${bare} resolves to ${(e as Error).message}`);
  }
}
