//! Synthetic continuous profiles, uploaded every minute like a Datadog profiler does through the
//! Agent: `POST /api/v2/profile` with an `event` JSON part and gzip pprof attachments. Java, .NET
//! and Python services send one `auto.pprof` (cpu-time, wall-time, alloc-space, heap-space); Go
//! services send `cpu.pprof` and `delta-heap.pprof` with the Go runtime's sample types. Samples in
//! a request carry the `trace endpoint` label, so the endpoint breakdown works. storefront-api has a
//! CPU burst now and then, which Profiles → Insights picks up.

use std::collections::HashMap;
use std::io::Write;

use crate::Rng;

mod pprof {
    #[derive(Clone, PartialEq, prost::Message)]
    pub struct Profile {
        #[prost(message, repeated, tag = "1")]
        pub sample_type: Vec<ValueType>,
        #[prost(message, repeated, tag = "2")]
        pub sample: Vec<Sample>,
        #[prost(message, repeated, tag = "4")]
        pub location: Vec<Location>,
        #[prost(message, repeated, tag = "5")]
        pub function: Vec<Function>,
        #[prost(string, repeated, tag = "6")]
        pub string_table: Vec<String>,
        #[prost(int64, tag = "9")]
        pub time_nanos: i64,
        #[prost(int64, tag = "10")]
        pub duration_nanos: i64,
    }
    #[derive(Clone, PartialEq, prost::Message)]
    pub struct ValueType {
        #[prost(int64, tag = "1")]
        pub r#type: i64,
        #[prost(int64, tag = "2")]
        pub unit: i64,
    }
    #[derive(Clone, PartialEq, prost::Message)]
    pub struct Sample {
        #[prost(uint64, repeated, tag = "1")]
        pub location_id: Vec<u64>,
        #[prost(int64, repeated, tag = "2")]
        pub value: Vec<i64>,
        #[prost(message, repeated, tag = "3")]
        pub label: Vec<Label>,
    }
    #[derive(Clone, PartialEq, prost::Message)]
    pub struct Label {
        #[prost(int64, tag = "1")]
        pub key: i64,
        #[prost(int64, tag = "2")]
        pub str: i64,
    }
    #[derive(Clone, PartialEq, prost::Message)]
    pub struct Location {
        #[prost(uint64, tag = "1")]
        pub id: u64,
        #[prost(message, repeated, tag = "4")]
        pub line: Vec<Line>,
    }
    #[derive(Clone, PartialEq, prost::Message)]
    pub struct Line {
        #[prost(uint64, tag = "1")]
        pub function_id: u64,
    }
    #[derive(Clone, PartialEq, prost::Message)]
    pub struct Function {
        #[prost(uint64, tag = "1")]
        pub id: u64,
        #[prost(int64, tag = "2")]
        pub name: i64,
        #[prost(int64, tag = "4")]
        pub filename: i64,
    }
}

/// One code path: frames root first as `function@file`, the endpoint it runs in ("" = background),
/// and its share of the service's CPU.
type Path = (&'static [&'static str], &'static str, f64);

struct Profiled {
    service: &'static str,
    host: &'static str,
    language: &'static str,
    version: &'static str,
    /// Typical CPU use, in cores.
    cores: f64,
    /// Typical live heap, in bytes.
    heap: f64,
    paths: &'static [Path],
}

const STOREFRONT: &[Path] = &[
    (
        &[
            "java.lang.Thread.run@Thread.java",
            "org.eclipse.jetty.util.thread.QueuedThreadPool$Runner.run@QueuedThreadPool.java",
            "org.springframework.web.servlet.DispatcherServlet.doDispatch@DispatcherServlet.java",
            "com.example.storefront.cart.CartController.getCart@CartController.java",
            "com.example.storefront.cart.CartService.load@CartService.java",
            "com.example.storefront.cart.PriceCalculator.total@PriceCalculator.java",
        ],
        "GET /cart",
        0.24,
    ),
    (
        &[
            "java.lang.Thread.run@Thread.java",
            "org.eclipse.jetty.util.thread.QueuedThreadPool$Runner.run@QueuedThreadPool.java",
            "org.springframework.web.servlet.DispatcherServlet.doDispatch@DispatcherServlet.java",
            "com.example.storefront.cart.CartController.getCart@CartController.java",
            "com.example.storefront.cart.CartService.load@CartService.java",
            "com.fasterxml.jackson.databind.ObjectMapper.readValue@ObjectMapper.java",
        ],
        "GET /cart",
        0.12,
    ),
    (
        &[
            "java.lang.Thread.run@Thread.java",
            "org.eclipse.jetty.util.thread.QueuedThreadPool$Runner.run@QueuedThreadPool.java",
            "org.springframework.web.servlet.DispatcherServlet.doDispatch@DispatcherServlet.java",
            "com.example.storefront.checkout.CheckoutController.checkout@CheckoutController.java",
            "com.example.storefront.checkout.OrderClient.submit@OrderClient.java",
            "com.fasterxml.jackson.databind.ObjectMapper.writeValueAsBytes@ObjectMapper.java",
        ],
        "POST /checkout",
        0.16,
    ),
    (
        &[
            "java.lang.Thread.run@Thread.java",
            "org.eclipse.jetty.util.thread.QueuedThreadPool$Runner.run@QueuedThreadPool.java",
            "org.springframework.web.servlet.DispatcherServlet.doDispatch@DispatcherServlet.java",
            "com.example.storefront.checkout.CheckoutController.checkout@CheckoutController.java",
            "com.example.storefront.checkout.PromotionEngine.apply@PromotionEngine.java",
            "java.util.regex.Pattern$Start.match@Pattern.java",
        ],
        "POST /checkout",
        0.14,
    ),
    (
        &[
            "java.lang.Thread.run@Thread.java",
            "org.eclipse.jetty.util.thread.QueuedThreadPool$Runner.run@QueuedThreadPool.java",
            "org.springframework.web.servlet.DispatcherServlet.doDispatch@DispatcherServlet.java",
            "com.example.storefront.catalog.ProductController.search@ProductController.java",
            "com.example.storefront.catalog.SearchIndex.query@SearchIndex.java",
        ],
        "GET /products",
        0.18,
    ),
    (&["java.lang.Thread.run@Thread.java", "com.zaxxer.hikari.pool.HouseKeeper.run@HouseKeeper.java"], "", 0.04),
    (&["java.lang.Thread.run@Thread.java", "G1 Young Generation@"], "", 0.12),
];

const CATALOG: &[Path] = &[
    (
        &[
            "System.Threading.ThreadPoolWorkQueue.Dispatch@",
            "Microsoft.AspNetCore.Server.Kestrel.Core.Internal.Http.HttpProtocol.ProcessRequests@",
            "Example.Catalog.Controllers.ProductsController.Get@ProductsController.cs",
            "Example.Catalog.Data.ProductRepository.FindAsync@ProductRepository.cs",
            "Npgsql.NpgsqlDataReader.Read@",
        ],
        "GET /api/products/{id}",
        0.34,
    ),
    (
        &[
            "System.Threading.ThreadPoolWorkQueue.Dispatch@",
            "Microsoft.AspNetCore.Server.Kestrel.Core.Internal.Http.HttpProtocol.ProcessRequests@",
            "Example.Catalog.Controllers.ProductsController.List@ProductsController.cs",
            "System.Text.Json.JsonSerializer.Serialize@",
        ],
        "GET /api/products",
        0.38,
    ),
    (&["System.Threading.ThreadPoolWorkQueue.Dispatch@", "Example.Catalog.Indexing.IndexRefresher.Run@IndexRefresher.cs", "Example.Catalog.Indexing.Tokenizer.Split@Tokenizer.cs"], "", 0.18),
    (&["System.GC.Collect@"], "", 0.1),
];

const WORKER: &[Path] = &[
    (
        &[
            "<module>@/app/worker.py",
            "run@/app/worker.py",
            "consume@/usr/local/lib/python3.12/site-packages/kombu/messaging.py",
            "handle_message@/app/handlers.py",
            "render_email@/app/templates.py",
            "render@/usr/local/lib/python3.12/site-packages/jinja2/environment.py",
        ],
        "",
        0.36,
    ),
    (
        &[
            "<module>@/app/worker.py",
            "run@/app/worker.py",
            "consume@/usr/local/lib/python3.12/site-packages/kombu/messaging.py",
            "handle_message@/app/handlers.py",
            "send@/app/smtp_client.py",
            "sendall@/usr/local/lib/python3.12/ssl.py",
        ],
        "",
        0.22,
    ),
    (
        &[
            "<module>@/app/worker.py",
            "run@/app/worker.py",
            "consume@/usr/local/lib/python3.12/site-packages/kombu/messaging.py",
            "decode@/usr/local/lib/python3.12/site-packages/kombu/serialization.py",
            "loads@/usr/local/lib/python3.12/json/__init__.py",
        ],
        "",
        0.18,
    ),
    (&["<module>@/app/worker.py", "run@/app/worker.py", "heartbeat@/usr/local/lib/python3.12/site-packages/kombu/connection.py"], "", 0.08),
    (&["<module>@/app/worker.py", "serve@/app/health.py", "handle@/usr/local/lib/python3.12/http/server.py"], "GET /healthz", 0.16),
];

const GATEWAY: &[Path] = &[
    (
        &[
            "runtime.goexit@runtime/asm_amd64.s",
            "net/http.(*conn).serve@net/http/server.go",
            "github.com/example-org/api-gateway/proxy.(*Router).ServeHTTP@proxy/router.go",
            "github.com/example-org/api-gateway/auth.Verify@auth/jwt.go",
            "crypto/ecdsa.Verify@crypto/ecdsa/ecdsa.go",
        ],
        "GET /api/*",
        0.3,
    ),
    (
        &[
            "runtime.goexit@runtime/asm_amd64.s",
            "net/http.(*conn).serve@net/http/server.go",
            "github.com/example-org/api-gateway/proxy.(*Router).ServeHTTP@proxy/router.go",
            "net/http/httputil.(*ReverseProxy).ServeHTTP@net/http/httputil/reverseproxy.go",
            "io.copyBuffer@io/io.go",
        ],
        "GET /api/*",
        0.32,
    ),
    (
        &[
            "runtime.goexit@runtime/asm_amd64.s",
            "net/http.(*conn).serve@net/http/server.go",
            "github.com/example-org/api-gateway/proxy.(*Router).ServeHTTP@proxy/router.go",
            "github.com/example-org/api-gateway/ratelimit.(*Limiter).Allow@ratelimit/limiter.go",
        ],
        "POST /api/*",
        0.14,
    ),
    (&["runtime.goexit@runtime/asm_amd64.s", "runtime.gcBgMarkWorker@runtime/mgc.go", "runtime.scanobject@runtime/mgcmark.go"], "", 0.18),
    (&["runtime.goexit@runtime/asm_amd64.s", "runtime.bgsweep@runtime/mgcsweep.go"], "", 0.06),
];

const SERVICES: &[Profiled] = &[
    Profiled { service: "storefront-api", host: "i-0a1b2c3d4e5f60002", language: "jvm", version: "2.14.0", cores: 0.9, heap: 610.0 * 1048576.0, paths: STOREFRONT },
    Profiled { service: "catalog-service", host: "i-0a1b2c3d4e5f60004", language: "dotnet", version: "1.8.3", cores: 0.35, heap: 240.0 * 1048576.0, paths: CATALOG },
    Profiled { service: "notification-worker", host: "i-0a1b2c3d4e5f60005", language: "python", version: "0.9.1", cores: 0.12, heap: 95.0 * 1048576.0, paths: WORKER },
    Profiled { service: "api-gateway", host: "i-0a1b2c3d4e5f60001", language: "go", version: "3.2.0", cores: 0.45, heap: 70.0 * 1048576.0, paths: GATEWAY },
];

/// pprof strings and frames, interned.
#[derive(Default)]
struct Builder {
    strings: Vec<String>,
    index: HashMap<String, i64>,
    functions: Vec<pprof::Function>,
    locations: Vec<pprof::Location>,
    frames: HashMap<&'static str, u64>,
}

impl Builder {
    fn new() -> Self {
        let mut b = Builder::default();
        b.s("");
        b
    }

    fn s(&mut self, v: &str) -> i64 {
        if let Some(&i) = self.index.get(v) {
            return i;
        }
        let i = self.strings.len() as i64;
        self.strings.push(v.to_string());
        self.index.insert(v.to_string(), i);
        i
    }

    /// Location id of a `function@file` frame.
    fn frame(&mut self, f: &'static str) -> u64 {
        if let Some(&id) = self.frames.get(f) {
            return id;
        }
        let (name, file) = f.split_once('@').unwrap_or((f, ""));
        let id = self.frames.len() as u64 + 1;
        let (name, filename) = (self.s(name), self.s(file));
        self.functions.push(pprof::Function { id, name, filename });
        self.locations.push(pprof::Location { id, line: vec![pprof::Line { function_id: id }] });
        self.frames.insert(f, id);
        id
    }

    fn finish(mut self, types: &[(&str, &str)], samples: Vec<(Vec<u64>, Vec<i64>, &'static str)>, start_ns: i64) -> Vec<u8> {
        let sample_type = types.iter().map(|(t, u)| pprof::ValueType { r#type: self.s(t), unit: self.s(u) }).collect();
        let key = self.s("trace endpoint");
        let sample = samples
            .into_iter()
            .map(|(location_id, value, endpoint)| {
                let label = if endpoint.is_empty() { vec![] } else { vec![pprof::Label { key, str: self.s(endpoint) }] };
                pprof::Sample { location_id, value, label }
            })
            .collect();
        let p = pprof::Profile { sample_type, sample, location: self.locations, function: self.functions, string_table: self.strings, time_nanos: start_ns, duration_nanos: 60_000_000_000 };
        let mut gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
        gz.write_all(&prost::Message::encode_to_vec(&p)).expect("gzip to memory");
        gz.finish().expect("gzip to memory")
    }
}

/// The attachments of one minute of a service: (file name, gzip pprof).
fn attachments(svc: &Profiled, rng: &mut Rng, minute: u64, start_ns: i64) -> Vec<(&'static str, Vec<u8>)> {
    // storefront-api runs hot for one minute in every 17.
    let burst = if svc.service == "storefront-api" && minute % 17 == 5 { 4.0 } else { 1.0 };
    let cores = svc.cores * burst * (0.85 + rng.f() * 0.3);
    let heap = svc.heap * (0.95 + rng.f() * 0.1);
    let mut b = Builder::new();
    let mut cpu = Vec::new();
    let mut mem = Vec::new();
    for (frames, endpoint, share) in svc.paths {
        // Leaf first.
        let stack: Vec<u64> = frames.iter().rev().map(|f| b.frame(f)).collect();
        let jitter = 0.8 + rng.f() * 0.4;
        let cpu_ns = (cores * share * jitter * 60e9) as i64;
        // Requests wait on I/O: wall time well above CPU time on endpoints.
        let wall_ns = cpu_ns * if endpoint.is_empty() { 1 } else { 3 } + (rng.f() * 2e9) as i64;
        let alloc = (cpu_ns as f64 * 0.8 * jitter) as i64;
        let live = (heap * share) as i64;
        cpu.push((stack.clone(), cpu_ns, wall_ns, *endpoint));
        mem.push((stack, alloc, live, *endpoint));
    }
    if svc.language == "go" {
        let mut heap_b = Builder::new();
        // The heap profile has its own string table.
        let heap_samples = mem.iter().map(|(stack, alloc, live, ep)| (remap(&b, &mut heap_b, stack), vec![alloc / 512, *alloc, live / 512, *live], *ep)).collect();
        let cpu_samples = cpu.into_iter().map(|(stack, ns, _, ep)| (stack, vec![ns / 10_000_000, ns], ep)).collect();
        return vec![
            ("cpu.pprof", b.finish(&[("samples", "count"), ("cpu", "nanoseconds")], cpu_samples, start_ns)),
            ("delta-heap.pprof", heap_b.finish(&[("alloc_objects", "count"), ("alloc_space", "bytes"), ("inuse_objects", "count"), ("inuse_space", "bytes")], heap_samples, start_ns)),
        ];
    }
    let samples = cpu.into_iter().zip(mem).map(|((stack, ns, wall, ep), (_, alloc, live, _))| (stack, vec![ns, wall, alloc, live], ep)).collect();
    vec![("auto.pprof", b.finish(&[("cpu-time", "nanoseconds"), ("wall-time", "nanoseconds"), ("alloc-space", "bytes"), ("heap-space", "bytes")], samples, start_ns))]
}

/// A stack of `from`'s location ids, as location ids of `to`.
fn remap(from: &Builder, to: &mut Builder, stack: &[u64]) -> Vec<u64> {
    stack.iter().map(|id| from.frames.iter().find(|(_, v)| *v == id).map(|(f, _)| to.frame(f)).unwrap_or(0)).collect()
}

/// RFC 3339 (UTC) from epoch nanoseconds.
fn rfc3339(ns: i64) -> String {
    let secs = ns.div_euclid(1_000_000_000);
    let (days, rem) = (secs.div_euclid(86_400), secs.rem_euclid(86_400));
    // Civil date from days (Howard Hinnant's algorithm).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!("{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z", rem / 3600, rem % 3600 / 60, rem % 60)
}

/// Uploads one minute of profiles (ending now) for every profiled service.
pub async fn send(http: &reqwest::Client, url: &str, rng: &mut Rng, now_ns: u64) {
    let end = now_ns as i64;
    let start = end - 60_000_000_000;
    let minute = now_ns / 60_000_000_000;
    for svc in SERVICES {
        let files = attachments(svc, rng, minute, start);
        let event = serde_json::json!({
            "attachments": files.iter().map(|(n, _)| *n).collect::<Vec<_>>(),
            "start": rfc3339(start),
            "end": rfc3339(end),
            "family": svc.language,
            "version": "4",
            "tags_profiler": format!("service:{},env:prod,version:{},host:{},runtime-id:{}-0001,profiler_version:1.0.0", svc.service, svc.version, svc.host, svc.service),
        });
        let mut form =
            reqwest::multipart::Form::new().part("event", reqwest::multipart::Part::bytes(event.to_string().into_bytes()).file_name("event.json").mime_str("application/json").expect("valid mime"));
        for (name, data) in files {
            form = form.part(name, reqwest::multipart::Part::bytes(data).file_name(name).mime_str("application/octet-stream").expect("valid mime"));
        }
        match http.post(url).multipart(form).send().await.and_then(|r| r.error_for_status()) {
            Ok(_) => tracing::debug!(service = svc.service, "profile sent"),
            Err(e) => tracing::warn!(service = svc.service, error = %e, "profile upload failed"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dates() {
        assert_eq!(rfc3339(0), "1970-01-01T00:00:00Z");
        assert_eq!(rfc3339(1_793_007_080_500_000_000), "2026-10-26T09:31:20Z");
    }

    #[test]
    fn go_services_split_cpu_and_heap() {
        let mut rng = Rng(7);
        let files = attachments(&SERVICES[3], &mut rng, 1, 0);
        assert_eq!(files.iter().map(|(n, _)| *n).collect::<Vec<_>>(), ["cpu.pprof", "delta-heap.pprof"]);
        assert_eq!(attachments(&SERVICES[0], &mut rng, 1, 0).len(), 1);
    }
}
