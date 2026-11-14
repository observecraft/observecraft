// How to add the tags each tagging convention needs, shown in the tagging policy editor and on the coverage page.
// Our own snippets (MIT), for the demo repositories (example-org/*).
import type { TagTarget } from '../../api/tagging/types';

export interface TagSnippet {
  key: string;
  title: string;
  targets: TagTarget[];
  /** What the snippet adds and where it goes. */
  summary: string;
  language: string;
  code: string;
}

export const SNIPPETS: TagSnippet[] = [
  {
    key: 'terraform-aws',
    title: 'Terraform on AWS: provider default_tags',
    targets: ['cloud'],
    summary: 'The AWS provider adds default_tags to every resource it creates. Set them once per root module; tf_path is the module’s directory in the repository.',
    language: 'hcl',
    code: `provider "aws" {
  region = "eu-west-1"

  default_tags {
    tags = {
      managed-by = "terraform"
      tf_repo    = "github.com/example-org/infra-terraform"
      tf_path    = "envs/prod/storage"
      team       = "storefront"
      env        = "prod"
    }
  }
}`,
  },
  {
    key: 'terraform-azure',
    title: 'Terraform on Azure: a merged locals map',
    targets: ['cloud'],
    summary: 'The azurerm provider has no default tags: keep them in locals and merge them into each resource’s tags.',
    language: 'hcl',
    code: `locals {
  default_tags = {
    managed-by = "terraform"
    tf_repo    = "github.com/example-org/infra-terraform"
    tf_path    = "envs/azure-prod/exports"
    team       = "billing"
    env        = "prod"
  }
}

resource "azurerm_storage_account" "exports" {
  name                     = "shopexports"
  resource_group_name      = azurerm_resource_group.shop.name
  location                 = "westeurope"
  account_tier             = "Standard"
  account_replication_type = "GRS"

  tags = merge(local.default_tags, { purpose = "exports" })
}`,
  },
  {
    key: 'yor-gitlab',
    title: 'Yor in GitLab CI',
    targets: ['cloud'],
    summary: 'Yor (Apache-2.0) adds git_repo, git_file, git_commit and yor_trace to every taggable resource of the Terraform files; run it before plan, or commit its changes.',
    language: 'yaml',
    code: `tag-resources:
  stage: validate
  image:
    name: bridgecrew/yor  # pin the release you use
    entrypoint: [""]
  script:
    - yor tag --directory . --tag-groups git,code2cloud --skip-tags git_last_modified_at,git_last_modified_by,git_modifiers
    - git diff --stat
  rules:
    - if: $CI_PIPELINE_SOURCE == "merge_request_event"
      changes: ["**/*.tf"]`,
  },
  {
    key: 'argocd-tracking',
    title: 'Argo CD: annotation tracking',
    targets: ['kubernetes'],
    summary:
      'With annotation tracking Argo CD writes argocd.argoproj.io/tracking-id on every object it applies. Collect the Applications with the Cluster Agent so their repository, path and revision are known.',
    language: 'yaml',
    code: `# argocd-cm
data:
  application.resourceTrackingMethod: annotation

# Datadog Helm chart values: send Argo CD Applications with the other manifests
datadog:
  orchestratorExplorer:
    enabled: true
    customResources:
      - argoproj.io/v1alpha1/applications`,
  },
  {
    key: 'docker-oci',
    title: 'Images: OCI source and revision labels',
    targets: ['image'],
    summary: 'Label the image with the repository and commit it was built from; the Datadog Agent reads them too (as git.repository_url and git.commit.sha).',
    language: 'shell',
    code: `docker build \\
  --label org.opencontainers.image.source="https://github.com/example-org/storefront-api" \\
  --label org.opencontainers.image.revision="$(git rev-parse HEAD)" \\
  -t registry.example.com/storefront-api:2.4.0 .`,
  },
  {
    key: 'unified-service-tagging',
    title: 'Services: unified service tagging and git metadata',
    targets: ['service', 'kubernetes'],
    summary: 'The tracers send service, env and version, and the repository and commit from DD_GIT_*. Set them in the deployment.',
    language: 'yaml',
    code: `metadata:
  labels:
    tags.datadoghq.com/service: storefront-api
    tags.datadoghq.com/env: prod
    tags.datadoghq.com/version: "2.4.0"
spec:
  template:
    spec:
      containers:
        - name: storefront-api
          env:
            - name: DD_SERVICE
              valueFrom: { fieldRef: { fieldPath: "metadata.labels['tags.datadoghq.com/service']" } }
            - name: DD_ENV
              valueFrom: { fieldRef: { fieldPath: "metadata.labels['tags.datadoghq.com/env']" } }
            - name: DD_VERSION
              valueFrom: { fieldRef: { fieldPath: "metadata.labels['tags.datadoghq.com/version']" } }
            - name: DD_GIT_REPOSITORY_URL
              value: https://github.com/example-org/storefront-api
            - name: DD_GIT_COMMIT_SHA
              value: "9c1e7b2"  # set by CI at deploy time`,
  },
];

/** The snippets for a target, in display order. */
export const snippetsFor = (target: TagTarget): TagSnippet[] => SNIPPETS.filter((s) => s.targets.includes(target));
