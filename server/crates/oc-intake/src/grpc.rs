//! OTLP/gRPC: the three OTLP `Export` services. Messages are passed through as bytes and posted to
//! the collector's OTLP/HTTP endpoint as protobuf (the same bytes), so only gRPC framing,
//! compression and status codes are handled here.

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
            let target = format!("{}{path}", st.cfg.collector_url);
            let protobuf = HeaderValue::from_static("application/x-protobuf");
            let up = send(&st, &org, &target, Some(&protobuf), None, body).await.map_err(rejected)?;
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
