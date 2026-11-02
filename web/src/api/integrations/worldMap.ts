// The countries a geomap widget draws (public/maps/world.json, web/scripts/build-world-map.mjs): fetched once.
/** The GeoJSON the map holds: countries with their name and ISO 3166-1 alpha-2 code. */
export interface WorldMap {
  type: 'FeatureCollection';
  features: { type: 'Feature'; properties: { name: string; iso: string }; geometry: unknown }[];
}

let world: Promise<WorldMap> | undefined;

export function loadWorldMap(): Promise<WorldMap> {
  world ??= fetch('/maps/world.json').then((r) => {
    if (!r.ok) throw new Error(`The world map didn't load (HTTP ${r.status})`);
    return r.json() as Promise<WorldMap>;
  });
  return world;
}
