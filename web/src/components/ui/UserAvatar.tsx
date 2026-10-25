import { memo } from 'react';

function hash(s: string): number {
  let h = 2166136261;
  for (let i = 0; i < s.length; i++) h = Math.imul(h ^ s.charCodeAt(i), 16777619);
  return h >>> 0;
}

/** Deterministic mirrored 5×5 pixel identicon in a circle, derived from the user id. */
export const UserAvatar = memo(function UserAvatar({ id, name, size = 24 }: { id: string; name?: string; size?: number }) {
  const h = hash(id);
  const hue = h % 360;
  const fg = `hsl(${hue} 70% 52%)`;
  const bg = `hsl(${(hue + 160) % 360} 55% 28%)`;
  const cells: [number, number][] = [];
  for (let y = 0; y < 5; y++) {
    for (let x = 0; x < 3; x++) {
      if ((h >> (y * 3 + x)) & 1) {
        cells.push([x, y]);
        if (x < 2) cells.push([4 - x, y]);
      }
    }
  }
  return (
    <svg width={size} height={size} viewBox="0 0 7 7" className="user-avatar" role="img" aria-label={name ?? 'User'}>
      {name && <title>{name}</title>}
      <circle cx="3.5" cy="3.5" r="3.5" fill={bg} />
      <g clipPath="circle(3.5px at 3.5px 3.5px)">
        {cells.map(([x, y]) => (
          <rect key={`${x}-${y}`} x={x + 1} y={y + 1} width="1.02" height="1.02" fill={fg} />
        ))}
      </g>
    </svg>
  );
});

/** Overlapping avatars with a "+n" counter. */
export function AvatarStack({ users, total, size = 28 }: { users: { id: string; name: string }[]; total: number; size?: number }) {
  const extra = total - users.length;
  return (
    <span className="avatar-stack">
      {users.map((u) => (
        <UserAvatar key={u.id} id={u.id} name={u.name} size={size} />
      ))}
      {extra > 0 && (
        <span className="avatar-more" style={{ width: size, height: size }}>
          +{extra}
        </span>
      )}
    </span>
  );
}
