# Amazon ECS → ObserveCraft

## Logs: FireLens

`firelens-task-definition.json` routes a container's stdout/stderr through FireLens (AWS for Fluent Bit)
to the intake's JSON endpoint `/api/v2/logs`. With `enable-ecs-log-metadata`, each record carries
`ecs_cluster`, `ecs_task_arn`, `ecs_task_definition` and `container_name`; ObserveCraft maps them to
Datadog's tags (`ecs_cluster`, `task_family`, `task_version`, `container_name`) and uses the task family
as the service unless the log says otherwise. JSON lines in `log` are parsed like any JSON log.

Store the header in Secrets Manager as the full value `authorization Bearer ocik_…` (the `Header`
option of Fluent Bit's http output), referenced by `secretOptions`.

Try it without AWS (ObserveCraft running locally):

```bash
docker run --rm -e OBSERVECRAFT_API_KEY=ocik_… \
  -v $PWD/local-firelens-test.conf:/fluent-bit/etc/fluent-bit.conf:ro fluent/fluent-bit:3.1.9
```

## Metrics, traces and ECS metadata: AWS Distro for OpenTelemetry sidecar

`adot-collector.yaml` (store it in SSM Parameter Store) and `adot-sidecar-container.json` (add it to the
task's `containerDefinitions`) run the ADOT Collector next to your containers:

- task and container CPU, memory, network and storage from the task metadata endpoint
  (`awsecscontainermetrics`),
- ECS resource attributes on every record (`resourcedetection` with the `ecs` detector),
- OTLP traces, metrics and logs your applications send to `localhost:4317/4318`.
