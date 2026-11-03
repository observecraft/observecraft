# Implementation plan: Cloud Cost Management from the providers' bills

Costs come only from custom costs files today. Datadog reads the providers' bills: an AWS Cost and Usage Report in an
S3 bucket (`datadog_aws_cur_config`) and Azure's Actual and Amortized cost exports in a storage account
(`datadog_azure_uc_config`). ObserveCraft does the same: the configs are Datadog's endpoints (Terraform provider, API
clients), and a job reads each billing month's latest files into `otel.cloud_costs`, where the cost metrics, the Cloud
Cost pages, dashboards, monitors and budgets already read them. Credentials are the integrations' own: the AWS
integration account of the config's `account_id` (its role or keys) and the Azure app registration of its `client_id`.

## Stage 1: The configs' API
Goal: `/api/v2/cost/aws_cur_config[/{id}]` and `/api/v2/cost/azure_uc_config[/{id}]` as `datadog-api-client-go`
describes them (numeric string ids; AWS PATCH answers a list, Azure's one pair; DELETE archives, 204; Azure `scope`
validated; timestamps with microseconds and no zone), in Postgres `cost_bill_configs`.
Success Criteria: the cassette scenarios of `cloud_cost_management.feature` for both resources replay; tf-compat
fixtures `aws_cur_config` and `azure_uc_config` apply, plan clean and destroy.
Status: Complete

## Stage 2: AWS bills
Goal: per active AWS config, every few hours, the manifests of the last months (CUR 2.0
`<prefix>/<export>/metadata/BILLING_PERIOD=YYYY-MM/…Manifest.json` and legacy CUR
`<prefix>/<report>/<yyyymmdd-yyyymmdd>/<report>-Manifest.json`) are read through the integration account's
credentials (S3, official AWS SDK); a month whose manifest changed is loaded (Parquet or gzip CSV, DuckDB) and
aggregated per day, account, service, region, charge type, resource and tags into FOCUS rows that replace that month
(`FileId` per config and month). Unblended → BilledCost; amortized (Savings Plans, Reserved Instances) →
EffectiveCost; public on-demand → ListCost. Status and `error_messages` on the config.
Success Criteria: unit tests on manifest discovery, column normalization (legacy `lineItem/UnblendedCost` and CUR 2.0
`line_item_unblended_cost`), amortization per line item type and month replacement, over small Parquet and CSV
fixtures written by the tests; a fake S3 for the job's run.
Status: Complete

## Stage 3: Azure bills
Goal: per active Azure config, the latest run's `manifest.json` per month of each export (actual, amortized), its
blobs read with the app registration (Storage Blob Data Reader, REST) and loaded the same way: Actual → BilledCost,
Amortized → EffectiveCost, per day, subscription, service, region, resource and tags.
Success Criteria: unit tests on manifest selection (latest run of a month), column mapping of the 2021-10-01 dataset
(CSV and Parquet) and the two datasets combined; a fake Blob service for the job's run.
Status: Complete

## Stage 4: Cloud Cost setup page
Goal: Cloud Cost → Settings lists the AWS and Azure configs with their status, last import and errors, and adds or
archives them (the same records); mocks for `make web-mock`; docs (`docs/datadog-api.md`, CLAUDE.md "Still missing").
Success Criteria: web typecheck; the page in the browser, live and mock.
Status: In Progress

## Stage 5: Lumenova (outside this repo)
Goal: terraform-aws-organization: a CUR 2.0 export in the management account to an S3 bucket, and S3 read on it for
the integration role; terraform-azure-organization: daily Actual and Amortized exports and Storage Blob Data Reader for
the app; terraform-observability: `datadog_aws_cur_config` and `datadog_azure_uc_config`.
Success Criteria: Cloud Cost shows Lumenova's AWS and Azure costs per service, account and region.
Status: Not Started
