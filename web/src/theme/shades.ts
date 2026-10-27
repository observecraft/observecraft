/** `n` shades of `hex`, from the color itself towards white (up to 60% lighter), for single-hue palettes. */
export function shades(hex: string, n: number): string[] {
  const rgb = [1, 3, 5].map((i) => parseInt(hex.slice(i, i + 2), 16));
  return Array.from({ length: n }, (_, i) => {
    const f = n > 1 ? (i * 0.6) / (n - 1) : 0;
    return `#${rgb
      .map((c) =>
        Math.round(c + (255 - c) * f)
          .toString(16)
          .padStart(2, '0'),
      )
      .join('')}`;
  });
}
