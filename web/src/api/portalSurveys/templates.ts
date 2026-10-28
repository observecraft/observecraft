// Starting points for new surveys. The questions are ObserveCraft's own.
import type { SurveyQuestion } from './types';

export interface SurveyTemplate {
  key: string;
  title: string;
  description: string;
  /** One line for the template picker. */
  summary: string;
  questions: SurveyQuestion[];
}

const likert = (id: string, title: string, required = true): SurveyQuestion => ({ id, kind: 'likert', title, description: '', required, options: [] });
const nps = (id: string, title: string): SurveyQuestion => ({ id, kind: 'nps', title, description: '', required: true, options: [] });
const text = (id: string, title: string): SurveyQuestion => ({ id, kind: 'text', title, description: '', required: false, options: [] });
const choice = (id: string, kind: 'single' | 'multi', title: string, options: string[]): SurveyQuestion => ({ id, kind, title, description: '', required: false, options });

export const SURVEY_TEMPLATES: SurveyTemplate[] = [
  {
    key: 'developer_experience',
    title: 'Developer Experience Survey',
    description: 'A quarterly check on how easy it is to build, test and ship here.',
    summary: 'Build speed, local setup, reviews and releases, plus an overall score.',
    questions: [
      likert('local_setup', 'I can get a service running on my machine without help'),
      likert('ci_speed', 'CI gives me feedback fast enough to stay focused'),
      likert('reviews', 'Code reviews happen quickly and are useful'),
      likert('release_confidence', 'I feel confident when my change goes to production'),
      choice('slowdowns', 'multi', 'What slows you down the most?', ['Flaky tests', 'Slow builds', 'Waiting for reviews', 'Unclear ownership', 'Environment setup', 'Missing documentation']),
      nps('recommend', 'How likely are you to recommend our engineering platform to another engineer?'),
      text('one_change', 'If you could change one thing about how we ship software, what would it be?'),
    ],
  },
  {
    key: 'onboarding',
    title: 'Engineering Onboarding Survey',
    description: 'For engineers who joined in the last 90 days: what helped and what got in the way.',
    summary: 'First commit, documentation, buddies and access for new joiners.',
    questions: [
      choice('first_commit', 'single', 'How long did it take to merge your first change?', ['Less than a day', '1–3 days', 'Up to a week', 'More than a week']),
      likert('docs_helpful', 'The documentation answered most of my early questions'),
      likert('access', 'I had the accounts and permissions I needed in my first week'),
      likert('buddy', 'My onboarding buddy or team helped me get productive'),
      choice('missing', 'multi', 'What was missing or hard to find?', ['Architecture overview', 'Service ownership', 'Deployment process', 'On-call expectations', 'Team rituals']),
      text('advice', 'What should we change for the next person who joins?'),
    ],
  },
  {
    key: 'platform_satisfaction',
    title: 'Platform Satisfaction Survey',
    description: 'How well the internal platform (CI/CD, environments, observability) serves product teams.',
    summary: 'Pipelines, environments, observability and support from the platform team.',
    questions: [
      likert('pipelines', 'Our CI/CD pipelines are reliable'),
      likert('environments', 'I can get a test environment when I need one'),
      likert('observability', 'I can find the logs, traces and metrics I need during an incident'),
      likert('support', 'The platform team responds quickly when I need help'),
      choice('most_used', 'single', 'Which platform capability do you rely on most?', [
        'Service templates',
        'Deployment pipelines',
        'Preview environments',
        'Dashboards and monitors',
        'Self-service actions',
      ]),
      nps('platform_nps', 'How likely are you to recommend the platform to a new team?'),
      text('wishlist', 'What should the platform team build next?'),
    ],
  },
];

export const templateByKey = (key: string | null | undefined): SurveyTemplate | undefined => SURVEY_TEMPLATES.find((t) => t.key === key);
