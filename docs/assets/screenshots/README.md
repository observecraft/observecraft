# README screenshots

These captures show ObserveCraft's mock UI with fictional commerce data. They illustrate the interface;
the displayed counts and timings are not production measurements or benchmarks.

To refresh them, run `make web-mock`, open the UI, and use a browser viewport of **1600 x 1000 CSS pixels**
with a **device pixel ratio of 1**. Wait for fonts, charts and tables to finish rendering, then export a
viewport screenshot as PNG without browser chrome or developer tools. Keep the filenames below so the
root README and organization profile continue to resolve their images.

| File | Route and state |
|---|---|
| `apm-services.png` | `/apm`: recommendations and service list |
| `trace-explorer.png` | `/apm/traces`: expand Requests, Errors, and Latency; pause live updates |
| `trace-view.png` | `/apm/traces`: filter to Error, open an API gateway trace, select a failing catalog-service database span, and show Waterfall with the error details |
| `monitors.png` | `/monitors`: populated monitor list with status filters visible |
| `infrastructure.png` | `/infrastructure`: select the AWS host filter |
| `ci-overview.png` | `/ci`: pipeline KPIs, executions chart and pipeline lists |

Inspect every exported image at its native resolution before committing. Use only fictional services,
hosts, repositories and people; never capture credentials or real customer telemetry.
