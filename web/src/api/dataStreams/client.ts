// Data Streams Monitoring data access. In "live" mode answers come from the Rust API (messaging
// spans and Kafka metrics in ClickHouse); otherwise the in-browser mock answers.
//   POST /api/v1/data-streams/map      streamMap      (services, topics, edges, pathways)
//   POST /api/v1/data-streams/measure  streamMeasure  (latency between two services over time)
//   POST /api/v1/data-streams/node     streamNode     (one node's messages and consumer lag over time)
//   POST /api/v1/data-streams/kafka    kafkaConsole   (clusters, topics, brokers, consumer groups, connectors)
import type { components } from '../generated/schema';
import { API_MODE, post } from '../http';
import * as mock from '../mock/dataStreams';

const live = API_MODE === 'live';
type S = components['schemas'];

export type StreamMap = S['StreamMap'];
export type StreamNode = S['StreamNode'];
export type StreamEdge = S['StreamEdge'];
export type StreamPathway = S['StreamPathway'];
export type StreamMapRequest = S['StreamMapRequest'];
export type StreamMeasure = S['StreamMeasure'];
export type StreamMeasureRequest = S['StreamMeasureRequest'];
export type StreamLatencyPoint = S['StreamLatencyPoint'];
export type StreamNodeDetail = S['StreamNodeDetail'];
export type StreamNodeRequest = S['StreamNodeRequest'];
export type StreamLagSeries = S['StreamLagSeries'];
export type KafkaConsole = S['KafkaConsole'];
export type KafkaConsoleRequest = S['KafkaConsoleRequest'];
export type KafkaCluster = S['KafkaCluster'];
export type KafkaTopic = S['KafkaTopic'];
export type KafkaBroker = S['KafkaBroker'];
export type KafkaConsumerGroup = S['KafkaConsumerGroup'];
export type KafkaGroupPartition = S['KafkaGroupPartition'];
export type KafkaGroupTopic = S['KafkaGroupTopic'];
export type KafkaConnector = S['KafkaConnector'];
export type KafkaClusterChart = S['KafkaClusterChart'];
export type KafkaHealthCheck = S['KafkaHealthCheck'];
export type KafkaCheckStatus = S['KafkaCheckStatus'];
export type KafkaChange = S['KafkaChange'];

export async function streamMap(req: StreamMapRequest): Promise<StreamMap> {
  if (!live) return mock.streamMap(req);
  return post('/api/v1/data-streams/map', req);
}

export async function streamMeasure(req: StreamMeasureRequest): Promise<StreamMeasure> {
  if (!live) return mock.streamMeasure(req);
  return post('/api/v1/data-streams/measure', req);
}

export async function streamNode(req: StreamNodeRequest): Promise<StreamNodeDetail> {
  if (!live) return mock.streamNode(req);
  return post('/api/v1/data-streams/node', req);
}

export async function kafkaConsole(req: KafkaConsoleRequest): Promise<KafkaConsole> {
  if (!live) return mock.kafkaConsole(req);
  return post('/api/v1/data-streams/kafka', req);
}
