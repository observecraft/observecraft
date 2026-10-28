// How AWS Lambda telemetry reaches ObserveCraft: the Datadog Lambda extension (enhanced metrics, traces and
// logs, the same layers and libraries as for Datadog, pointed at this intake) or OpenTelemetry's Lambda layers.
// {{intake}} and {{apiKey}} are filled in per install.
import type { SetupOption } from '../setup/SetupOptions';

export const SERVERLESS_SETUP: SetupOption[] = [
  {
    id: 'datadog-extension',
    name: 'Datadog Lambda extension',
    summary:
      "Datadog's open-source Lambda extension runs next to each function and sends enhanced metrics (invocations, errors, cold starts, duration, memory, estimated cost), traces from the Datadog Lambda library and the function's logs.",
    steps: [
      {
        title: 'Add the extension and the library layers',
        text: 'Add the Datadog extension layer and the Datadog Lambda library layer for the function runtime, the same way you would for Datadog (console, SAM, CDK, Serverless Framework or Terraform). Set the handler as the library documents for your runtime.',
      },
      {
        title: 'Point the extension at ObserveCraft',
        text: 'Set these environment variables on the function. Nothing is sent to Datadog: the site is a placeholder and every endpoint is this intake.',
        code: 'DD_API_KEY={{apiKey}}\nDD_SITE=observecraft.invalid\nDD_DD_URL={{intake}}\nDD_APM_DD_URL={{intake}}\nDD_LOGS_CONFIG_LOGS_DD_URL=<intake host>:4318\nDD_LOGS_CONFIG_FORCE_USE_HTTP=true\nDD_SERVERLESS_LOGS_ENABLED=true\nDD_ENHANCED_METRICS=true\nDD_TRACE_ENABLED=true\nDD_ENV=prod\nDD_SERVICE=checkout\nDD_VERSION=1.0.0',
      },
      {
        title: 'Tag the functions',
        text: 'The extension tags every metric with functionname, region, account_id, memorysize, runtime and cold_start. Add team and stage tags (DD_TAGS=team:payments,stage:prod) to fill the Teams list and the Stage facet.',
      },
      { title: 'Check the data', text: 'After the next invocations the functions appear here, grouped by service, with their insights and estimated cost.' },
    ],
  },
  {
    id: 'aws-integration',
    name: 'aws.lambda.* metrics',
    summary:
      'Functions without the extension still appear from the aws.lambda.* metrics (invocations, errors, throttles, duration) if something forwards the AWS CloudWatch Lambda metrics under those names, for example a metric stream converted to Datadog series.',
    steps: [
      { title: 'Send the series to the Datadog series API of this intake', code: 'POST {{intake}}/api/v2/series\nDD-API-KEY: {{apiKey}}' },
      { title: 'Keep the Lambda tags', text: 'Each series needs functionname and region tags (and aws_account, memorysize, runtime when known) to be matched to its function.' },
    ],
  },
  {
    id: 'otel-lambda',
    name: 'OpenTelemetry Lambda',
    summary: "OpenTelemetry's Lambda layers instrument the function and export spans (with faas.* attributes) and FaaS metrics over OTLP.",
    steps: [
      {
        title: 'Add the OpenTelemetry layers',
        text: 'Add the OpenTelemetry Lambda layer for your runtime and the Collector layer, and set AWS_LAMBDA_EXEC_WRAPPER as the layer documents.',
      },
      {
        title: 'Export to ObserveCraft',
        text: 'Configure the Collector layer (collector.yaml in the function package) with this exporter, and point OPENTELEMETRY_COLLECTOR_CONFIG_URI at it.',
        code: 'exporters:\n  otlphttp/observecraft:\n    endpoint: {{intake}}\n    headers:\n      authorization: "Bearer {{apiKey}}"\nservice:\n  pipelines:\n    traces:\n      receivers: [otlp]\n      exporters: [otlphttp/observecraft]\n    metrics:\n      receivers: [otlp]\n      exporters: [otlphttp/observecraft]',
      },
      { title: 'Check the data', text: 'Functions are identified by faas.name and cloud.region; cold starts come from faas.coldstart and memory from faas.max_memory.' },
    ],
  },
];
