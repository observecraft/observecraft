// TanStack Query bindings for Integrations → Slack.
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import {
  addSlackChannel,
  connectSlackAccount,
  disconnectSlackAccount,
  getSlackOverview,
  listSlackConversations,
  removeSlackChannel,
  startSlackOAuth,
  testSlackChannel,
  updateSlackChannel,
  type SlackDisplay,
} from '../../../api/slack/client';

const slackKeys = { all: ['slack'] as const, overview: () => [...slackKeys.all, 'overview'] as const };

export function useSlackOverview() {
  return useQuery({ queryKey: slackKeys.overview(), queryFn: getSlackOverview });
}

/** The workspace's channels the bot can see, for the channel picker. */
export function useSlackConversations(accountId: string) {
  return useQuery({ queryKey: [...slackKeys.all, 'conversations', accountId], queryFn: () => listSlackConversations(accountId), staleTime: 60_000, retry: false });
}

/** Writes; the overview and the notification channels (Monitors → Settings) are refetched afterwards. */
export function useSlackMutations() {
  const qc = useQueryClient();
  const onSuccess = () =>
    Promise.all([qc.invalidateQueries({ queryKey: slackKeys.overview() }), qc.invalidateQueries({ queryKey: ['notification-channels'] }), qc.invalidateQueries({ queryKey: ['integrations'] })]);
  return {
    connect: useMutation({ mutationFn: connectSlackAccount, onSuccess }),
    oauth: useMutation({ mutationFn: startSlackOAuth }),
    disconnect: useMutation({ mutationFn: disconnectSlackAccount, onSuccess }),
    addChannel: useMutation({ mutationFn: ({ accountId, name, display }: { accountId: string; name: string; display: SlackDisplay }) => addSlackChannel(accountId, name, display), onSuccess }),
    updateChannel: useMutation({ mutationFn: ({ id, ...input }: { id: string; name?: string; display?: SlackDisplay }) => updateSlackChannel(id, input), onSuccess }),
    removeChannel: useMutation({ mutationFn: removeSlackChannel, onSuccess }),
    testChannel: useMutation({ mutationFn: testSlackChannel }),
  };
}
