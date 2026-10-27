// ObserveCraft-authored guidance. Upstream prose/artwork is not republished.
export function dashboardNote(content, title, section = '') {
  const text = content.toLowerCase();
  const heading = /^(?:#{1,3})\s+([^\n]+)/m.exec(content)?.[1]?.trim();
  if (/requests and limits/.test(text))
    return '## Resource requests and limits\n\nRequests influence scheduling. Limits constrain resource use: CPU can be throttled, and exceeding a memory limit can terminate a container.\n\nCompare sustained use and peak use before changing a workload.\n\n[Resource configuration](https://kubernetes.io/docs/concepts/configuration/manage-resources-containers/)';
  if (/how to use this dashboard/.test(text))
    return '## Explore capacity\n\n- Choose a cluster and a time range that includes normal traffic and peaks.\n- Compare requested CPU and memory with observed use.\n- Select a workload to inspect its behavior over time.';
  if (/how to use this section/.test(text))
    return '## Compare a workload\n\nSelect a deployment or daemonset above. Compare environments and clusters, then narrow the filters to investigate a difference.';
  if (/utilization per kubernetes object/.test(text))
    return '## Workload utilization\n\nEach row identifies a workload. Read resource requests alongside measured CPU and memory use; a low average alone does not establish that a request is excessive.';
  if (/cluster capacity review/.test(text))
    return '## Cluster capacity\n\nInspect reserved resources and remaining capacity by cluster. Positive idle CPU and unused requested memory indicate a gap between reservations and measured use.';
  if (/spikes|stable through time/.test(text))
    return '## Check variability\n\nLook for bursts and sustained load in the adjacent charts. Compare peaks with typical use before reducing a resource request.';
  if (/right request/.test(text))
    return '## Review request sizing\n\nUse observed utilization as evidence for a request change. Keep room for bursts, and check application latency and restarts after a change.';
  if (/capacity planning/.test(text))
    return '## Kubernetes capacity planning\n\nFind workloads whose resource reservations differ from observed use. Review CPU and memory separately, then inspect workload history before resizing.';
  if (/kubernetes monitoring guides/.test(text))
    return '## Kubernetes overview\n\nStart with node health and available capacity, then inspect pod readiness and resource use.\n\n[Cluster architecture](https://kubernetes.io/docs/concepts/architecture/)\n\n[Resource metrics](https://kubernetes.io/docs/tasks/debug/debug-cluster/resource-metrics-pipeline/)';
  if (/overview of your kubernetes/.test(text))
    return '## Read this dashboard\n\nInfrastructure panels summarize cluster and node capacity. Pod panels describe scheduling and readiness. Network and disk charts show activity over the selected interval.\n\nUse the filters above to keep every panel focused on the same resources.';
  const label = heading ?? section ?? title;
  return `## ${label || title}\n\nInspect ${section || title} over the selected interval. Use cluster, namespace and workload filters to narrow the results. Empty panels indicate no matching telemetry for that selection.`;
}
