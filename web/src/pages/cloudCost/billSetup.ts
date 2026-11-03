// Cloud Cost → Settings: what the provider pages show about a bill config, and what the connect flows
// build from the form (the S3 read policy, the Azure scope). Pure, tested by scripts/cloud-cost-bills.test.mjs.
import type { BillConfig } from '../../api/cloudCost/client';

/** An export path prefix as AWS writes it: no leading or trailing '/'. */
export const validPrefix = (prefix: string) => !prefix.startsWith('/') && !prefix.endsWith('/');

/** The IAM statement letting the integration role read the Cost and Usage Report bucket. */
export function curBucketPolicy(bucket: string) {
  const name = bucket.trim() || '<BUCKET_NAME>';
  return {
    Version: '2012-10-17',
    Statement: [
      { Sid: 'ObserveCraftCURList', Effect: 'Allow', Action: ['s3:ListBucket'], Resource: `arn:aws:s3:::${name}` },
      { Sid: 'ObserveCraftCURRead', Effect: 'Allow', Action: ['s3:GetObject'], Resource: `arn:aws:s3:::${name}/*` },
    ],
  };
}

export type AzureScopeKind = 'subscription' | 'billing-account';

/** The scope of Azure's exports: a subscription or a billing account. */
export function azureScope(kind: AzureScopeKind, id: string): string {
  const v = id.trim();
  return kind === 'subscription' ? `/subscriptions/${v}` : `/providers/Microsoft.Billing/billingAccounts/${v}`;
}

export type BillState = 'active' | 'disabled' | 'error';

/** The config's error messages (both exports for Azure). */
export function billErrors(c: BillConfig): string[] {
  const d = c.doc;
  if (d.type === 'aws_cur_config') return d.attributes.error_messages ?? [];
  return d.attributes.configs.flatMap((e) => (e.error_messages ?? []).map((m) => `${e.dataset_type}: ${m}`));
}

/** ERROR when the last import reported errors, else ACTIVE or DISABLED. */
export function billState(c: BillConfig): BillState {
  const d = c.doc;
  const disabled = d.type === 'aws_cur_config' ? d.attributes.status === 'disabled' : d.attributes.configs.every((e) => e.status === 'disabled');
  if (disabled) return 'disabled';
  return billErrors(c).length ? 'error' : 'active';
}

/** What the config reads: the account and where the bill files are. */
export function billSource(c: BillConfig): { account: string; location: string } {
  const d = c.doc;
  if (d.type === 'aws_cur_config') {
    const a = d.attributes;
    return { account: a.account_id, location: `s3://${[a.bucket_name, a.report_prefix, a.report_name].filter(Boolean).join('/')}` };
  }
  const [first] = d.attributes.configs;
  if (!first) return { account: '', location: '' };
  const places = [...new Set(d.attributes.configs.map((e) => `${e.storage_account}/${e.storage_container}`))];
  return { account: `${first.account_id} · ${first.scope}`, location: places.join(', ') };
}

/** The billing months read (Azure's two exports count a month once). */
export function monthsRead(c: BillConfig): number {
  return new Set(c.imported_months.map((m) => m.slice(m.indexOf(':') + 1))).size;
}
