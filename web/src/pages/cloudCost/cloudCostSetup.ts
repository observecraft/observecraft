// How costs reach ObserveCraft: custom costs files (FOCUS line items) uploaded through the Cloud Cost API,
// the same call as Datadog's (PUT /api/v2/cost/custom_costs). Cloud providers' bills aren't read yet.
import type { SetupOption } from '../setup/SetupOptions';

const EXAMPLE = `[
  {
    "ProviderName": "aws",
    "ChargeDescription": "Amazon EC2 m6i.large",
    "ChargePeriodStart": "2026-10-01",
    "ChargePeriodEnd": "2026-11-01",
    "BilledCost": 1840.25,
    "BillingCurrency": "USD",
    "Tags": { "team": "checkout", "service": "order-service", "env": "prod" }
  }
]`;

/** The setup steps, with this install's address in the upload command. */
export function cloudCostSetup(origin: string): SetupOption[] {
  return [
    {
      id: 'custom-costs',
      name: 'Custom costs file',
      summary:
        "Upload costs as a JSON list of line items, from any provider or SaaS vendor. Each line item is spread evenly over the days of its charge period. Reading AWS, Azure and Google Cloud bills directly isn't available yet: export them and upload the line items.",
      steps: [
        {
          title: 'Write the line items',
          text: 'One object per charge. ProviderName becomes the provider (aws, azure, gcp or any vendor name); Tags become cost tags you can filter and allocate by.',
          code: EXAMPLE,
        },
        {
          title: 'Upload the file',
          text: 'With one of your application keys (Personal Settings → Application Keys). GET on the same path lists the uploaded files.',
          code: `curl -X PUT "${origin}/api/v2/cost/custom_costs" \\\n  -H "DD-API-KEY: {{apiKey}}" \\\n  -H "DD-APPLICATION-KEY: <APP_KEY>" \\\n  -H "Content-Type: application/json" \\\n  --data @costs.json`,
        },
        { title: 'Check the costs', text: 'The overview fills in right away, and the costs can be queried as all.cost and <provider>.cost.amortized in dashboards, monitors and budgets.' },
      ],
    },
  ];
}
