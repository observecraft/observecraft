// Blueprints: pre-filled self-service actions offered on the Actions tab. Choosing one opens the
// editor with its form and step; the user adds the credentials it names and adjusts hosts and
// defaults before saving. Every blueprint passes the same checks as a saved action (logic.ts).
import type { ActionInput, ActionStep, PortalActionCategory } from './types';

export type Blueprint = {
  key: string;
  title: string;
  description: string;
  category: PortalActionCategory;
  /** Brand logo key (BrandLogo / IntegrationLogo registry), or `internal` for ObserveCraft steps. */
  logo: string;
  suggested?: boolean;
  inputs: ActionInput[];
  step: ActionStep;
  /** Secrets the step refers to, with what to put in them. */
  secrets: { name: string; hint: string }[];
};

const text = (key: string, label: string, extra: Partial<ActionInput> = {}): ActionInput => ({ key, label, kind: 'text', required: true, options: [], default: '', description: '', ...extra });
const select = (key: string, label: string, options: string[], def = options[0]): ActionInput => ({ key, label, kind: 'select', required: true, options, default: def, description: '' });

const GITHUB = 'https://api.github.com';
const GITHUB_HEADERS = [
  { name: 'Authorization', value: 'Bearer {{secrets.github_token}}' },
  { name: 'Accept', value: 'application/vnd.github+json' },
  { name: 'X-GitHub-Api-Version', value: '2022-11-28' },
];
const GITHUB_SECRET = { name: 'github_token', hint: 'A fine-grained token with access to the repositories (contents, pull requests, actions)' };
const GITLAB_HEADERS = [{ name: 'PRIVATE-TOKEN', value: '{{secrets.gitlab_token}}' }];
const GITLAB_SECRET = { name: 'gitlab_token', hint: 'A project or group access token with the api scope' };
const json = (v: unknown) => JSON.stringify(v, null, 2);

export const BLUEPRINTS: Blueprint[] = [
  {
    key: 'github_scaffold',
    title: 'Scaffold a new project in GitHub',
    description:
      'Create a repository from one of your template repositories, so every new service starts with the same layout, CI workflows and ownership files. Developers pick a name and visibility; the action does the rest and links to the new repository.',
    category: 'scaffolding',
    logo: 'github',
    suggested: true,
    inputs: [
      text('template_owner', 'Template owner', { default: 'example-org' }),
      text('template_repo', 'Template repository', { default: 'service-template' }),
      text('owner', 'New repository owner', { default: 'example-org' }),
      text('name', 'Repository name', { description: 'Lowercase, with dashes: storefront-search' }),
      text('description', 'Description', { required: false }),
      { key: 'private', label: 'Private repository', kind: 'boolean', required: false, options: [], default: 'true', description: '' },
    ],
    step: {
      kind: 'http',
      requests: [
        {
          method: 'POST',
          url: `${GITHUB}/repos/{{inputs.template_owner}}/{{inputs.template_repo}}/generate`,
          headers: GITHUB_HEADERS,
          body: json({ owner: '{{inputs.owner}}', name: '{{inputs.name}}', description: '{{inputs.description}}', private: '{{inputs.private}}' }),
        },
      ],
    },
    secrets: [GITHUB_SECRET],
  },
  {
    key: 'gitlab_scaffold',
    title: 'Scaffold a new project in GitLab',
    description:
      'Start a GitLab project from a template project in your group, with its pipelines, merge request rules and docs already in place. The new project lands in the namespace you choose and the run links straight to it.',
    category: 'scaffolding',
    logo: 'gitlab',
    inputs: [
      text('name', 'Project name'),
      { key: 'namespace_id', label: 'Namespace (group) id', kind: 'number', required: true, options: [], default: '', description: 'Numeric id of the group the project belongs to' },
      { key: 'template_project_id', label: 'Template project id', kind: 'number', required: true, options: [], default: '', description: '' },
      select('visibility', 'Visibility', ['private', 'internal', 'public']),
    ],
    step: {
      kind: 'http',
      requests: [
        {
          method: 'POST',
          url: 'https://gitlab.com/api/v4/projects',
          headers: GITLAB_HEADERS,
          body: json({
            name: '{{inputs.name}}',
            namespace_id: '{{inputs.namespace_id}}',
            template_project_id: '{{inputs.template_project_id}}',
            use_custom_template: true,
            visibility: '{{inputs.visibility}}',
          }),
        },
      ],
    },
    secrets: [GITLAB_SECRET],
  },
  {
    key: 'github_workflow',
    title: 'Trigger a GitHub Actions workflow',
    description:
      'Start a workflow_dispatch run of a deployment or maintenance workflow without granting write access to the repository. Developers choose the branch and environment; the workflow keeps its own approvals and environment protections.',
    category: 'release_management',
    logo: 'githubactions',
    suggested: true,
    inputs: [
      text('owner', 'Repository owner', { default: 'example-org' }),
      text('repo', 'Repository', { default: 'storefront-api' }),
      text('workflow', 'Workflow file', { default: 'deploy.yml' }),
      text('ref', 'Branch or tag', { default: 'main' }),
      select('environment', 'Environment', ['staging', 'production']),
    ],
    step: {
      kind: 'http',
      requests: [
        {
          method: 'POST',
          url: `${GITHUB}/repos/{{inputs.owner}}/{{inputs.repo}}/actions/workflows/{{inputs.workflow}}/dispatches`,
          headers: GITHUB_HEADERS,
          body: json({ ref: '{{inputs.ref}}', inputs: { environment: '{{inputs.environment}}' } }),
        },
      ],
    },
    secrets: [GITHUB_SECRET],
  },
  {
    key: 'gitlab_pipeline',
    title: 'Trigger a GitLab pipeline',
    description:
      'Run a project pipeline on a branch with the variables your jobs expect, for releases, data backfills or one-off maintenance. The run links to the pipeline so developers can follow its jobs in GitLab.',
    category: 'release_management',
    logo: 'gitlab',
    inputs: [
      text('project', 'Project path or id', { default: 'example-org/order-service' }),
      text('ref', 'Branch or tag', { default: 'main' }),
      select('environment', 'Environment', ['staging', 'production']),
    ],
    step: {
      kind: 'http',
      requests: [
        {
          method: 'POST',
          url: 'https://gitlab.com/api/v4/projects/{{inputs.project}}/pipeline',
          headers: GITLAB_HEADERS,
          body: json({ ref: '{{inputs.ref}}', variables: [{ key: 'DEPLOY_ENV', value: '{{inputs.environment}}' }] }),
        },
      ],
    },
    secrets: [GITLAB_SECRET],
  },
  {
    key: 'jira_issue',
    title: 'Create a Jira issue',
    description:
      'File a ticket in the right project with a consistent summary, type and description, and the requester recorded automatically. Point the URL at your Jira site; the run links to the new issue.',
    category: 'incident_management',
    logo: 'jira',
    inputs: [
      text('project', 'Project key', { default: 'SHOP' }),
      select('issue_type', 'Issue type', ['Task', 'Bug', 'Story']),
      text('summary', 'Summary'),
      text('description', 'Description', { required: false }),
    ],
    step: {
      kind: 'http',
      requests: [
        {
          method: 'POST',
          url: 'https://your-site.atlassian.net/rest/api/3/issue',
          headers: [
            { name: 'Authorization', value: 'Basic {{secrets.jira_credentials | base64}}' },
            { name: 'Accept', value: 'application/json' },
          ],
          body: json({
            fields: {
              project: { key: '{{inputs.project}}' },
              issuetype: { name: '{{inputs.issue_type}}' },
              summary: '{{inputs.summary}}',
              description: { type: 'doc', version: 1, content: [{ type: 'paragraph', content: [{ type: 'text', text: '{{inputs.description}} (requested by {{user.email}})' }] }] },
            },
          }),
        },
      ],
    },
    secrets: [{ name: 'jira_credentials', hint: 'email:api-token of the account that files the issues' }],
  },
  {
    key: 'terraform_pr',
    title: 'Open a pull request to a Terraform repository',
    description:
      'Propose infrastructure changes the reviewed way: the action branches off the default branch, commits the Terraform file you provide and opens a pull request, so plans and approvals run in your usual pipeline.',
    category: 'infrastructure_provisioning',
    logo: 'terraform',
    suggested: true,
    inputs: [
      text('owner', 'Repository owner', { default: 'example-org' }),
      text('repo', 'Repository', { default: 'infrastructure' }),
      text('base', 'Base branch', { default: 'main' }),
      text('branch', 'New branch', { description: 'For example add-orders-bucket' }),
      text('path', 'File path', { default: 'envs/staging/main.tf' }),
      text('content', 'Terraform', { description: 'The file content to commit' }),
      text('title', 'Pull request title'),
    ],
    step: {
      kind: 'http',
      requests: [
        { method: 'GET', url: `${GITHUB}/repos/{{inputs.owner}}/{{inputs.repo}}/git/ref/heads/{{inputs.base}}`, headers: GITHUB_HEADERS, body: '' },
        {
          method: 'POST',
          url: `${GITHUB}/repos/{{inputs.owner}}/{{inputs.repo}}/git/refs`,
          headers: GITHUB_HEADERS,
          body: json({ ref: 'refs/heads/{{inputs.branch}}', sha: '{{steps.1.object.sha}}' }),
        },
        {
          method: 'PUT',
          url: `${GITHUB}/repos/{{inputs.owner}}/{{inputs.repo}}/contents/{{inputs.path | path}}`,
          headers: GITHUB_HEADERS,
          body: json({ message: '{{inputs.title}}', content: '{{inputs.content | base64}}', branch: '{{inputs.branch}}' }),
        },
        {
          method: 'POST',
          url: `${GITHUB}/repos/{{inputs.owner}}/{{inputs.repo}}/pulls`,
          headers: GITHUB_HEADERS,
          body: json({ title: '{{inputs.title}}', head: '{{inputs.branch}}', base: '{{inputs.base}}', body: 'Opened by {{user.name}} from the Developer Portal' }),
        },
      ],
    },
    secrets: [GITHUB_SECRET],
  },
  {
    key: 'declare_incident',
    title: 'Declare an incident',
    description:
      'Give every developer a one-step way to declare an incident with the right severity and owning team, without learning the incident tool first. The incident opens in Incident Management with the requester as commander.',
    category: 'incident_management',
    logo: 'internal',
    inputs: [
      text('title', 'What is happening?'),
      select('severity', 'Severity', ['SEV-1', 'SEV-2', 'SEV-3', 'SEV-4', 'SEV-5'], 'SEV-3'),
      text('team', 'Owning team', { required: false }),
      text('summary', 'Details', { required: false }),
    ],
    step: { kind: 'create_incident', fields: { title: '{{inputs.title}}', severity: '{{inputs.severity}}', teams: '{{inputs.team}}', summary: '{{inputs.summary}}' } },
    secrets: [],
  },
  {
    key: 'page_on_call',
    title: 'Page an on-call team',
    description:
      "Reach the team that's on call for a service through its escalation policy, with a title and context the responder can act on. High urgency pages notify right away; low urgency ones follow the responders' preferences.",
    category: 'incident_management',
    logo: 'internal',
    inputs: [
      text('team', 'Team handle', { default: 'checkout' }),
      text('title', 'Why are you paging?'),
      text('details', 'Details', { required: false }),
      select('urgency', 'Urgency', ['high', 'low']),
    ],
    step: { kind: 'page_on_call', fields: { team: '{{inputs.team}}', title: '{{inputs.title}}', description: '{{inputs.details}} (paged by {{user.name}})', urgency: '{{inputs.urgency}}' } },
    secrets: [],
  },
];

export const blueprint = (key: string | null | undefined) => BLUEPRINTS.find((b) => b.key === key);
