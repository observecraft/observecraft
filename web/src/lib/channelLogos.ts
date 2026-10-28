// Brand logos for notification channels and the tools that send data, from the generated SVG Logos
// subset (CC0; see scripts/build-logos.mjs). Brands remain trademarks of their owners and are shown
// only to identify the integration.

/** Recipient handle prefix → logo key (`@slack-checkout-alerts` → slack). */
const RECIPIENT_PREFIX: [string, string][] = [
  ['slack-', 'slack'],
  ['pagerduty-', 'pagerduty'],
  ['opsgenie-', 'opsgenie'],
  ['webhook-', 'webhook'],
  ['teams-', 'teams'],
];

/** The logo of a notification recipient, or null for people, emails and ObserveCraft teams. */
export function recipientLogo(recipient: string): string | null {
  return RECIPIENT_PREFIX.find(([prefix]) => recipient.startsWith(prefix))?.[1] ?? null;
}

/** Tool logos used by setup pages (databases, data jobs). */
export const TOOL_LOGOS = ['postgresql', 'mysql', 'mongodb', 'redis', 'spark', 'apacheairflow', 'dbt', 'snowflake'];
