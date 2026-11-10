// In-browser Slack integration for mock mode (Integrations → Slack). Workspaces and channels live in
// memory for the session; names, handles and display defaults follow the server (routes/slack.rs).
import { DEFAULT_DISPLAY, slackHandle, type SlackAccount, type SlackChannel, type SlackConversation, type SlackDisplay, type SlackOverview } from '../slack/types';

const delay = <T>(value: T, ms = 150) => new Promise<T>((r) => setTimeout(() => r(structuredClone(value)), ms));
const fail = (message: string): never => {
  throw new Error(message);
};

const SCOPES = ['chat:write', 'chat:write.public', 'channels:read', 'groups:read', 'users:read', 'users:read.email'];
const ORIGIN = typeof window === 'undefined' ? 'https://observecraft.example.com' : window.location.origin;
const INTERACTIVITY = `${ORIGIN}/api/v1/public/slack/interactive`;

const MANIFEST = `display_information:
  name: ObserveCraft
  description: Monitor alerts, On-Call pages and reports from ObserveCraft
  background_color: "#1f2937"
features:
  bot_user:
    display_name: ObserveCraft
    always_online: true
oauth_config:
  scopes:
    bot:
${SCOPES.map((s) => `      - ${s}`).join('\n')}
settings:
  interactivity:
    is_enabled: true
    request_url: ${INTERACTIVITY}
  org_deploy_enabled: false
  socket_mode_enabled: false
  token_rotation_enabled: false
`;

const CONVERSATIONS: SlackConversation[] = [
  { id: 'C0A1B2C3D01', name: 'checkout-alerts', isPrivate: false, isMember: true },
  { id: 'C0A1B2C3D02', name: 'incidents', isPrivate: false, isMember: true },
  { id: 'C0A1B2C3D03', name: 'releases', isPrivate: false, isMember: false },
  { id: 'C0A1B2C3D04', name: 'storefront-oncall', isPrivate: false, isMember: false },
  { id: 'G0A1B2C3D05', name: 'billing-private', isPrivate: true, isMember: true },
];

function channel(account: string, name: string, display: Partial<SlackDisplay> = {}, conversationId: string | null = null): SlackChannel {
  return { id: crypto.randomUUID(), name, handle: slackHandle(account, name), display: { ...DEFAULT_DISPLAY, ...display }, conversationId, createdAt: Date.now() - 86_400_000 * 12 };
}

let accounts: SlackAccount[] | null = null;

function db(): SlackAccount[] {
  accounts ??= [
    {
      id: '5b6c7d8e-9f01-4a2b-8c3d-4e5f6a7b8c9d',
      name: 'example-shop',
      teamId: 'T0EXAMPLE01',
      teamName: 'Example Shop',
      botUserId: 'U0OBSCRAFT',
      installedVia: 'oauth',
      interactive: true,
      missingScopes: [],
      createdAt: Date.now() - 86_400_000 * 30,
      channels: [
        channel('example-shop', '#checkout-alerts', { mute_buttons: true }, 'C0A1B2C3D01'),
        channel('example-shop', '#incidents', {}, 'C0A1B2C3D02'),
        channel('example-shop', '#releases', { message: true, notified: false, tags: false }),
      ],
    },
  ];
  return accounts;
}

const account = (id: string) => db().find((a) => a.id === id) ?? fail('Slack account not found');
const owner = (channelId: string) => db().find((a) => a.channels.some((c) => c.id === channelId)) ?? fail('Slack channel not found');
const bare = (n: string) => n.trim().replace(/^#+/, '').toLowerCase();

function checkChannelName(raw: string): string {
  const n = raw.trim();
  const b = n.replace(/^#/, '');
  if (!b || b.length > 80 || !/^[\p{L}\p{N}._-]+$/u.test(b)) fail('Channel names are 1–80 letters, digits, dashes, underscores or periods, with an optional leading #');
  return n;
}

export const getSlackOverview = (): Promise<SlackOverview> =>
  delay({
    oauthAvailable: true,
    redirectUrl: `${ORIGIN}/api/v1/public/slack/oauth/callback`,
    interactivityUrl: INTERACTIVITY,
    appManifest: MANIFEST,
    scopes: SCOPES,
    accounts: db(),
  });

/** Whether a workspace is connected: the catalog tile is installed, as the server counts one. */
export const slackConnected = (): boolean => db().length > 0;

export function connectSlackAccount(input: { name: string; botToken: string; signingSecret?: string | null }): Promise<SlackAccount> {
  const name = input.name.trim().toLowerCase();
  if (!/^[a-z0-9_-]{1,60}$/.test(name)) fail('Account names are 1–60 lowercase letters, digits, dashes or underscores');
  if (!input.botToken.trim().startsWith('xoxb-')) fail("That isn't a bot token: use the Bot User OAuth Token (xoxb-…) from the app's OAuth & Permissions page");
  const existing = db().find((a) => a.name === name);
  const next: SlackAccount = {
    id: existing?.id ?? crypto.randomUUID(),
    name,
    teamId: existing?.teamId ?? 'T0EXAMPLE02',
    teamName: existing?.teamName ?? 'Example Shop Support',
    botUserId: 'U0OBSCRAFT2',
    installedVia: 'token',
    interactive: !!input.signingSecret || !!existing?.interactive,
    missingScopes: [],
    createdAt: existing?.createdAt ?? Date.now(),
    channels: existing?.channels ?? [],
  };
  accounts = [...db().filter((a) => a.id !== next.id), next];
  return delay(next, 400);
}

export function startSlackOAuth(name: string): Promise<{ url: string }> {
  if (!/^[a-z0-9_-]{1,60}$/.test(name.trim().toLowerCase())) fail('Account names are 1–60 lowercase letters, digits, dashes or underscores');
  // Mock mode has no Slack: come straight back as if the install succeeded.
  const n = name.trim().toLowerCase();
  if (!db().some((a) => a.name === n)) {
    accounts = [
      ...db(),
      {
        id: crypto.randomUUID(),
        name: n,
        teamId: 'T0EXAMPLE03',
        teamName: 'Example Shop Partners',
        botUserId: 'U0OBSCRAFT3',
        installedVia: 'oauth',
        interactive: true,
        missingScopes: [],
        createdAt: Date.now(),
        channels: [],
      },
    ];
  }
  return delay({ url: `/integrations/slack?connected=${encodeURIComponent(n)}` });
}

export function disconnectSlackAccount(id: string): Promise<void> {
  account(id);
  accounts = db().filter((a) => a.id !== id);
  return delay(undefined);
}

export const listSlackConversations = (id: string): Promise<SlackConversation[]> => (account(id), delay(CONVERSATIONS, 250));

export function addSlackChannel(accountId: string, name: string, display: SlackDisplay): Promise<SlackChannel> {
  const a = account(accountId);
  const n = checkChannelName(name);
  if (a.channels.some((c) => bare(c.name) === bare(n))) fail(`${n} is already configured for this Slack account`);
  const c = channel(a.name, n, display);
  c.createdAt = Date.now();
  a.channels = [...a.channels, c].sort((x, y) => bare(x.name).localeCompare(bare(y.name)));
  return delay(c);
}

export function updateSlackChannel(id: string, input: { name?: string; display?: SlackDisplay }): Promise<SlackChannel> {
  const a = owner(id);
  const c = a.channels.find((x) => x.id === id)!;
  if (input.name !== undefined) {
    const n = checkChannelName(input.name);
    if (a.channels.some((x) => x.id !== id && bare(x.name) === bare(n))) fail(`${n} is already configured for this Slack account`);
    if (bare(n) !== bare(c.name)) c.conversationId = null;
    c.name = n;
    c.handle = slackHandle(a.name, n);
  }
  if (input.display) c.display = { ...c.display, ...input.display };
  return delay(c);
}

export function removeSlackChannel(id: string): Promise<void> {
  const a = owner(id);
  a.channels = a.channels.filter((c) => c.id !== id);
  return delay(undefined);
}

export function testSlackChannel(id: string) {
  const a = owner(id);
  const c = a.channels.find((x) => x.id === id)!;
  return delay({ recipient: c.handle, status: 'sent', error: null }, 500);
}

/** The configured channels as notification channels (Monitors → Settings lists them). */
export const slackNotificationChannels = () =>
  db().flatMap((a) =>
    a.channels.map((c) => ({ id: c.id, handle: c.handle, kind: 'slack', target: `${a.teamName || a.name} · ${c.name}`, source: 'slack_app', signed: false, createdAt: c.createdAt })),
  );
