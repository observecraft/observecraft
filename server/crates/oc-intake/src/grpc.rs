//! OTLP/gRPC: the three OTLP `Export` services. Messages are passed through as bytes and posted to
//! the shared intake path as protobuf. That path stamps log identities before Kafka; this module
//! handles gRPC framing, compression and status codes.

use std::future::Future;
use std::pin::Pin;

use axum::body::Body;
use axum::extract::{Request, State};
use axum::http::{HeaderValue, StatusCode};
use axum::response::Response;
use axum::routing::post;
use axum::Router;
use bytes::{Buf, BufMut, Bytes};
use tonic::codec::{Codec, CompressionEncoding, DecodeBuf, Decoder, EncodeBuf, Encoder};
use tonic::server::{Grpc, UnaryService};
use tonic::{Code, Status};

use crate::{admit, send, Rejection, Shared};

/// Codec that leaves messages encoded.
#[derive(Clone, Copy, Default)]
struct Raw;

impl Encoder for Raw {
    type Item = Bytes;
    type Error = Status;
    fn encode(&mut self, item: Bytes, dst: &mut EncodeBuf<'_>) -> Result<(), Status> {
        dst.put(item);
        Ok(())
    }
}

impl Decoder for Raw {
    type Item = Bytes;
    type Error = Status;
    fn decode(&mut self, src: &mut DecodeBuf<'_>) -> Result<Option<Bytes>, Status> {
        Ok(Some(src.copy_to_bytes(src.remaining())))
    }
}

impl Codec for Raw {
    type Encode = Bytes;
    type Decode = Bytes;
    type Encoder = Raw;
    type Decoder = Raw;
    fn encoder(&mut self) -> Raw {
        Raw
    }
    fn decoder(&mut self) -> Raw {
        Raw
    }
}

/// gRPC status for an HTTP status (OTLP's mapping: throttling and outages are retryable).
fn code(status: StatusCode) -> Code {
    match status.as_u16() {
        400 => Code::InvalidArgument,
        401 => Code::Unauthenticated,
        403 => Code::PermissionDenied,
        404 => Code::Unimplemented,
        413 => Code::ResourceExhausted,
        429 | 502 | 503 | 504 => Code::Unavailable,
        _ => Code::Internal,
    }
}

fn rejected(r: Rejection) -> Status {
    Status::new(code(r.status), r.message)
}

/// One `Export` call, forwarded to `path` on the collector.
struct Export {
    st: Shared,
    path: &'static str,
}

impl UnaryService<Bytes> for Export {
    type Response = Bytes;
    type Future = Pin<Box<dyn Future<Output = Result<tonic::Response<Bytes>, Status>> + Send>>;

    fn call(&mut self, req: tonic::Request<Bytes>) -> Self::Future {
        let (st, path) = (self.st.clone(), self.path);
        Box::pin(async move {
            let headers = req.metadata().clone().into_headers();
            let body = req.into_inner();
            let org = admit(&st, &headers, body.len()).await.map_err(rejected)?;
            let target = match path {
                "/v1/traces" => crate::kafka::Sink::Traces,
                "/v1/metrics" => crate::kafka::Sink::Metrics,
                _ => crate::kafka::Sink::Logs,
            };
            let protobuf = HeaderValue::from_static("application/x-protobuf");
            let up = send(&st, &org, target, Some(&protobuf), None, body).await.map_err(rejected)?;
            if up.status.is_success() {
                // The ExportResponse message, partial success included.
                Ok(tonic::Response::new(up.body))
            } else {
                Err(Status::new(code(up.status), format!("collector answered HTTP {}", up.status.as_u16())))
            }
        })
    }
}

async fn export(st: Shared, path: &'static str, req: Request) -> Response {
    let max = st.cfg.max_body;
    let mut grpc = Grpc::new(Raw).accept_compressed(CompressionEncoding::Gzip).accept_compressed(CompressionEncoding::Zstd).send_compressed(CompressionEncoding::Gzip).max_decoding_message_size(max);
    grpc.unary(Export { st, path }, req).await.map(Body::new)
}

/// The gRPC listener's routes (served over HTTP/2 cleartext; TLS is the ingress' job).
pub fn router(st: Shared) -> Router {
    Router::new()
        .route("/opentelemetry.proto.collector.trace.v1.TraceService/Export", post(|State(st): State<Shared>, req: Request| export(st, "/v1/traces", req)))
        .route("/opentelemetry.proto.collector.metrics.v1.MetricsService/Export", post(|State(st): State<Shared>, req: Request| export(st, "/v1/metrics", req)))
        .route("/opentelemetry.proto.collector.logs.v1.LogsService/Export", post(|State(st): State<Shared>, req: Request| export(st, "/v1/logs", req)))
        .with_state(st)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::otlp::{self, ExportLogsServiceRequest, LogRecord, ResourceLogs, ScopeLogs};
    use prost::Message;
    use sha2::{Digest, Sha256};
    use std::{io::Write, time::Instant};
    use tower::ServiceExt;

    #[tokio::test]
    async fn grpc_logs_receive_server_identities_after_framing_and_decompression() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let mut cfg = crate::Config::from_env();
        cfg.collector_url = format!("http://{}", listener.local_addr().unwrap());
        let task = tokio::spawn(async move {
            axum::serve(
                listener,
                Router::new().route(
                    "/v1/logs",
                    post(|headers: axum::http::HeaderMap, body: Bytes| async move {
                        assert_eq!(headers["x-oc-org"], "example-org");
                        body
                    }),
                ),
            )
            .await
            .unwrap();
        });
        let st = crate::test_state(cfg);
        st.keys.lock().unwrap().insert(
            Sha256::digest(b"ocik_grpc_identity_test").to_vec(),
            crate::CachedKey {
                org: Some(crate::KeyOrg {
                    org_id: "example-org".into(),
                    span_retention_days: 15,
                    metrics_retention_days: 30,
                    metrics_5m_days: 90,
                    metrics_1h_days: 395,
                    flag_environment_id: None,
                }),
                at: Instant::now(),
                touched: Some(Instant::now()),
            },
        );
        let record = LogRecord {
            attributes: vec![otlp::kv("oc.ingest_id", "forged")],
            ..Default::default()
        };
        let input = ExportLogsServiceRequest {
            resource_logs: vec![ResourceLogs {
                resource: None,
                scope_logs: vec![ScopeLogs {
                    scope: None,
                    log_records: vec![record.clone(), record],
                }],
            }],
        }
        .encode_to_vec();
        let mut gzip = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        gzip.write_all(&input).unwrap();
        for (encoding, bytes) in [
            ("identity", input.clone()),
            ("gzip", gzip.finish().unwrap()),
            ("zstd", zstd::stream::encode_all(&input[..], 1).unwrap()),
        ] {
            let mut frame = vec![u8::from(encoding != "identity")];
            frame.extend_from_slice(&(bytes.len() as u32).to_be_bytes());
            frame.extend_from_slice(&bytes);
            let req = Request::builder()
                .method("POST")
                .uri("/opentelemetry.proto.collector.logs.v1.LogsService/Export")
                .version(axum::http::Version::HTTP_2)
                .header("content-type", "application/grpc")
                .header("grpc-encoding", encoding)
                .header("authorization", "Bearer ocik_grpc_identity_test")
                .body(Body::from(frame))
                .unwrap();
            let result = router(st.clone()).oneshot(req).await.unwrap();
            assert_eq!(result.status(), StatusCode::OK);
            let bytes = axum::body::to_bytes(result.into_body(), 1024 * 1024).await.unwrap();
            assert_eq!(bytes[0], 0);
            let length = u32::from_be_bytes(bytes[1..5].try_into().unwrap()) as usize;
            let result = ExportLogsServiceRequest::decode(&bytes[5..5 + length]).unwrap();
            let records = &result.resource_logs[0].scope_logs[0].log_records;
            let id = |r: &LogRecord| otlp::text(r.attributes.iter().find(|a| a.key == "oc.ingest_id").unwrap()).to_owned();
            assert_ne!(id(&records[0]), "forged");
            assert_ne!(id(&records[0]), id(&records[1]));
            assert_ne!(records[0].time_unix_nano, 0);
        }
        task.abort();
    }
}
