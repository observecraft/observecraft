// OpenTelemetry Collector configuration that sends one receiver's metrics to ObserveCraft.
// {{intake}} and {{apiKey}} are filled in per install (see `fill`).

/** A Collector config: `receiver` (its YAML under `receivers:`, indented by two) in a metrics pipeline to the intake. */
export function collectorConfig(name: string, receiver: string): string {
  return `receivers:
${receiver}
processors:
  resourcedetection:
    detectors: [env, system]
  batch: {}
exporters:
  otlphttp/observecraft:
    endpoint: {{intake}}
    headers:
      authorization: "Bearer {{apiKey}}"
service:
  pipelines:
    metrics/${name}:
      receivers: [${receiver.trim().split(':')[0]}]
      processors: [resourcedetection, batch]
      exporters: [otlphttp/observecraft]`;
}
