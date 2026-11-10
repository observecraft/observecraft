//! A connector package's manifest (`connector.yaml`): the contract package authors write against. The JSON Schema
//! `integrations/connectors/connector.schema.json` is generated from these types (`oc-connector schema`), so the
//! descriptions below are what editors show. Fields marked *template* take `{{ expr }}` parts (docs/connectors.md);
//! *durations* are `30s`, `5m`, `2h`, `1d`.
//!
//! Changes are additive: new optional fields, kinds and filters. A breaking change bumps `schema_version`.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use utoipa::ToSchema;

/// A connector package's manifest.
#[derive(Deserialize, Serialize, ToSchema, Clone, Debug)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    /// The manifest format: 1.
    pub schema_version: u32,
    /// The connector's id: lowercase letters, digits, `-`, `_`, `.`; the catalog tile's id when it implements one
    /// (`have-i-been-pwned`). Also its jobs' provider, so it never changes once published.
    pub id: String,
    /// The name shown in the catalog and on the connector page (`Have I Been Pwned`).
    pub name: String,
    /// Who wrote and maintains the package (`ObserveCraft` for bundled packages; a company or person otherwise).
    pub publisher: String,
    /// The vendor whose API is called.
    pub vendor: String,
    /// The package's own version (semver).
    #[serde(default = "default_version")]
    pub version: String,
    /// One or two sentences: what is collected and how.
    pub description: String,
    /// Catalog tile ids this connector makes available (`web/src/api/integrations/catalog.generated.json`); empty
    /// for a connector without a Datadog tile.
    #[serde(default)]
    pub tiles: Vec<String>,
    /// Logo name (`web/src/api/integrations/integrationLogos.generated.ts`); defaults to the first tile.
    #[serde(default)]
    pub logo: Option<String>,
    /// Catalog categories (`Security`, `Collaboration`, …).
    #[serde(default)]
    pub categories: Vec<String>,
    /// Logs' `source` (`ddsource`) and the source integration pipelines filter on; defaults to `id`.
    #[serde(default)]
    pub source: Option<String>,
    /// The vendor's API documentation.
    #[serde(default)]
    pub docs_url: Option<String>,
    /// Setup instructions shown above the form (Markdown: where to create the key, which role it needs).
    #[serde(default)]
    pub setup: Option<String>,
    /// Links shown with the setup (a consent URL, the vendor's key page); `url` is a template over `config`.
    #[serde(default)]
    pub links: Vec<Link>,
    /// What identifies the connected account or instance, as a template over `config` (`{{ config.base_url | host
    /// }}`): logs' `host`, `connection.instance`, part of every log's identity. Defaults to the connector id.
    #[serde(default)]
    pub instance: Option<String>,
    /// The setup form.
    #[serde(default)]
    pub config: Vec<ConfigField>,
    /// How requests authenticate (streams may override it). Default `none`.
    #[serde(default)]
    pub auth: Auth,
    /// A client certificate for mutual TLS.
    #[serde(default)]
    pub tls: Option<Tls>,
    /// Headers on every request (templates), before the stream's own.
    #[serde(default)]
    pub headers: BTreeMap<String, String>,
    /// Error rules for every request, after the stream's own.
    #[serde(default)]
    pub errors: Vec<ErrorRule>,
    /// Rate limits for every stream (a stream's own replaces it).
    #[serde(default)]
    pub rate_limit: Option<RateLimit>,
    /// A request quota for the whole connection (every stream and the test).
    #[serde(default)]
    pub budget: Option<Budget>,
    /// Static tables for mappings (`tables.event_types[record.type | str]`).
    #[serde(default)]
    #[schema(value_type = Value)]
    pub tables: BTreeMap<String, Value>,
    /// Requests run before the streams, cached for their `ttl`, readable as `discovery.<name>` everywhere after
    /// `auth` (a data region's base URL, the tenants of a partner account).
    #[serde(default)]
    pub discover: Vec<Discovery>,
    /// Requests sent once when a connection is saved (starting a subscription the API needs before it answers).
    #[serde(default)]
    pub on_connect: Vec<Call>,
    /// What is collected by polling (empty for a push-only package).
    #[serde(default)]
    pub streams: Vec<Stream>,
    /// The connection test (the page's "Test connection"); required with streams.
    #[serde(default)]
    pub test: Option<Test>,
    /// The vendor sends to ObserveCraft itself (a log drain, OTLP, a webhook): the page shows where to point it,
    /// and the package's assets are installed with Integrations → Install. No connection is stored.
    #[serde(default)]
    pub push: Option<Push>,
    /// Events the vendor pushes instead (an inbound webhook on the intake).
    #[serde(default)]
    pub webhook: Option<Webhook>,
    /// A native connector compiled into the server that runs the streams (non-HTTP sources: SQS and S3,
    /// WebSockets); its streams may then leave `request` out.
    #[serde(default)]
    pub native: Option<String>,
    /// Dashboards, log pipeline, monitors and detection rules shipped with the package.
    #[serde(default)]
    pub assets: Assets,
    /// Recorded answers for tests and the UI's mock mode.
    #[serde(default)]
    pub mock: Option<Mock>,
}

fn default_version() -> String {
    "1.0.0".into()
}

#[derive(Deserialize, Serialize, ToSchema, Clone, Debug)]
#[serde(deny_unknown_fields)]
pub struct Link {
    pub label: String,
    /// Template over `config`.
    pub url: String,
}

/// A setup form field. Non-secret fields are `config.<key>` in templates; secret ones `secrets.<key>`.
#[derive(Deserialize, Serialize, ToSchema, Clone, Debug)]
#[serde(deny_unknown_fields)]
pub struct ConfigField {
    /// `^[a-z][a-z0-9_]*$`.
    pub key: String,
    pub label: String,
    #[serde(rename = "type")]
    pub kind: FieldKind,
    #[serde(default)]
    pub required: bool,
    /// Sealed at rest, never answered, scrubbed from errors (string or text fields). A saved secret is only sent to
    /// the URLs it was saved with: changing a `url` field asks for every secret again.
    #[serde(default)]
    pub secret: bool,
    /// The value when left empty.
    #[serde(default)]
    #[schema(value_type = Value)]
    pub default: Option<Value>,
    #[serde(default)]
    pub placeholder: Option<String>,
    /// One line under the field.
    #[serde(default)]
    pub help: Option<String>,
    /// The choices of a select or multiselect.
    #[serde(default)]
    pub options: Vec<SelectOption>,
    /// A regular expression the value (each item of a list) must match.
    #[serde(default)]
    pub pattern: Option<String>,
    /// Suffixes a url field drops (`/api`), since the requests add them.
    #[serde(default)]
    pub strip_suffix: Vec<String>,
}

#[derive(Deserialize, Serialize, ToSchema, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum FieldKind {
    /// One line of text.
    String,
    /// Several lines (a JSON key file, a PEM certificate).
    Text,
    /// An https URL (http only when the server allows private hosts), checked against the SSRF policy.
    Url,
    Number,
    Boolean,
    Select,
    Multiselect,
    /// Several values, one per line (domains, project ids).
    List,
}

#[derive(Deserialize, Serialize, ToSchema, Clone, Debug)]
#[serde(deny_unknown_fields)]
pub struct SelectOption {
    pub value: String,
    pub label: String,
}

/// How a request authenticates. Templates here may read `secrets`.
#[derive(Deserialize, Serialize, ToSchema, Clone, Debug, Default)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Auth {
    /// No credentials.
    #[default]
    None,
    /// A key in a header or the query (`Authorization: apikey {{ config.email }}:{{ secrets.key }}` works too).
    ApiKey {
        #[serde(rename = "in", default)]
        location: KeyIn,
        /// The header or query parameter's name.
        name: String,
        /// Template.
        value: String,
    },
    /// `Authorization: Bearer <token>`.
    Bearer {
        /// Template.
        token: String,
    },
    /// HTTP basic authentication.
    Basic {
        /// Template.
        username: String,
        /// Template.
        password: String,
    },
    /// Several headers (templates).
    Headers { headers: BTreeMap<String, String> },
    /// OAuth 2.0 client credentials: the token is fetched, cached until it expires, and sent as a bearer token.
    Oauth2ClientCredentials(OAuthClient),
    /// OAuth 2.0 refresh token grant (a token the user created or consented to). A rotated refresh token is saved
    /// into the `rotate` secret.
    Oauth2RefreshToken(OAuthRefresh),
    /// An RS256 JWT assertion exchanged for a token (Google service accounts, customer-registered OAuth apps).
    JwtBearer(JwtBearer),
    /// A JWT signed with the connection's key and sent as the bearer token, renewed before it expires (Snowflake key
    /// pairs). `headers` (templates over `auth.jwt`) replace `Authorization: Bearer`.
    Jwt(JwtSpec),
    /// A login request whose answer gives a token (and other values, such as a base URL), used by the next
    /// requests as `auth.<name>`; cached for `ttl` (or the answer's expiry).
    TokenExchange(TokenExchange),
    /// A request signer compiled into the server: `duo_hmac_v2`, `duo_hmac_v5`, `cortex_xdr_advanced`,
    /// `trend_micro_apex_central`, `akamai_edgegrid`.
    Signer {
        signer: Signer,
        /// The signer's inputs (templates): Duo `ikey`, `skey`; Cortex XDR `key_id`, `key`; Apex Central
        /// `application_id`, `api_key`; EdgeGrid `client_token`, `client_secret`, `access_token`.
        params: BTreeMap<String, String>,
    },
}

#[derive(Deserialize, Serialize, ToSchema, Clone, Copy, Debug, Default, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum KeyIn {
    #[default]
    Header,
    Query,
}

#[derive(Deserialize, Serialize, ToSchema, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Signer {
    /// Duo Admin API, HMAC-SHA1 of date, method, host, path and sorted parameters.
    DuoHmacV2,
    /// Duo Admin API, HMAC-SHA512 (v5: with the body's SHA-512 and the `X-Duo-Date` header).
    DuoHmacV5,
    /// Palo Alto Cortex XDR "Advanced" keys: SHA-256 of key, nonce and timestamp in `Authorization`.
    CortexXdrAdvanced,
    /// Trend Micro Apex Central: an HS256 JWT per request with the request's SHA-256 checksum.
    TrendMicroApexCentral,
    /// Akamai EdgeGrid (`EG1-HMAC-SHA256`): `client_token`, `client_secret`, `access_token`.
    AkamaiEdgegrid,
}

#[derive(Deserialize, Serialize, ToSchema, Clone, Debug)]
#[serde(deny_unknown_fields)]
pub struct OAuthClient {
    /// Template.
    pub token_url: String,
    /// Template.
    pub client_id: String,
    /// Template.
    pub client_secret: String,
    #[serde(default)]
    pub scopes: Vec<String>,
    /// How the client authenticates to the token endpoint.
    #[serde(default)]
    pub client_auth: ClientAuth,
    /// More form parameters (`audience`, `resource`; templates).
    #[serde(default)]
    pub params: BTreeMap<String, String>,
}

#[derive(Deserialize, Serialize, ToSchema, Clone, Debug)]
#[serde(deny_unknown_fields)]
pub struct OAuthRefresh {
    /// Template.
    pub token_url: String,
    /// Template.
    pub client_id: String,
    /// Template; may be empty for public clients.
    #[serde(default)]
    pub client_secret: Option<String>,
    /// Template (usually `{{ secrets.refresh_token }}`).
    pub refresh_token: String,
    #[serde(default)]
    pub scopes: Vec<String>,
    #[serde(default)]
    pub client_auth: ClientAuth,
    #[serde(default)]
    pub params: BTreeMap<String, String>,
    /// The secret field a rotated refresh token is saved into.
    #[serde(default)]
    pub rotate: Option<String>,
}

#[derive(Deserialize, Serialize, ToSchema, Clone, Copy, Debug, Default, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ClientAuth {
    /// HTTP basic with the client id and secret.
    #[default]
    Basic,
    /// `client_id` and `client_secret` in the form.
    Body,
}

#[derive(Deserialize, Serialize, ToSchema, Clone, Debug)]
#[serde(deny_unknown_fields)]
pub struct JwtBearer {
    /// Template; defaults to the service account's `token_uri`.
    #[serde(default)]
    pub token_url: Option<String>,
    /// A Google service account JSON key (template): gives the private key, `iss` and the token URL.
    #[serde(default)]
    pub service_account: Option<String>,
    /// An RSA private key in PEM, PKCS#8 or PKCS#1 (template), when there is no service account.
    #[serde(default)]
    pub private_key: Option<String>,
    /// The JWT header's `kid` (template).
    #[serde(default)]
    pub key_id: Option<String>,
    /// Claims (templates; `iat` and `exp` are added): `iss`, `sub`, `aud`, …
    #[serde(default)]
    #[schema(value_type = Value)]
    pub claims: BTreeMap<String, Value>,
    /// Added as the `scope` claim, space separated.
    #[serde(default)]
    pub scopes: Vec<String>,
    /// How long an assertion is valid. Duration, default 1h.
    #[serde(default)]
    pub lifetime: Option<String>,
    /// More form parameters of the token request (templates).
    #[serde(default)]
    pub params: BTreeMap<String, String>,
}

#[derive(Deserialize, Serialize, ToSchema, Clone, Debug)]
#[serde(deny_unknown_fields)]
pub struct TokenExchange {
    /// The login request (templates may read `secrets`, and `auth.jwt` with `jwt`).
    pub request: Request,
    /// A JWT signed for the login request, as `auth.jwt` (GitHub Apps: the app's JWT gets an installation token).
    #[serde(default)]
    pub jwt: Option<JwtSpec>,
    /// Values taken from the answer (templates over `response`, `headers`): `token`, `base_url`, `cookie`, …
    pub capture: BTreeMap<String, String>,
    /// Seconds the values are valid, from the answer (template); else `ttl`.
    #[serde(default)]
    pub expires_in: Option<String>,
    /// Duration, default 1h.
    #[serde(default)]
    pub ttl: Option<String>,
    /// Headers sent on every request (templates over `auth`, `config`, `secrets`); default `Authorization: Bearer
    /// {{ auth.token }}`.
    #[serde(default)]
    pub headers: BTreeMap<String, String>,
}

/// A JWT the engine signs (RS256). Claims are templates; `iat` and `exp` are added; `key.fingerprint` (`SHA256:` and
/// the base64 SHA-256 of the public key, as Snowflake names keys) can be read in them.
#[derive(Deserialize, Serialize, ToSchema, Clone, Debug)]
#[serde(deny_unknown_fields)]
pub struct JwtSpec {
    /// An RSA private key in PEM, PKCS#8 or PKCS#1 (template, a secret).
    pub private_key: String,
    /// The header's `kid` (template).
    #[serde(default)]
    pub key_id: Option<String>,
    #[serde(default)]
    #[schema(value_type = Value)]
    pub claims: BTreeMap<String, Value>,
    /// Duration, default 1h (GitHub Apps: at most 10m).
    #[serde(default)]
    pub lifetime: Option<String>,
    /// `kind: jwt` only: the headers carrying it (templates over `auth.jwt`), instead of `Authorization: Bearer`.
    #[serde(default)]
    pub headers: BTreeMap<String, String>,
}

#[derive(Deserialize, Serialize, ToSchema, Clone, Debug)]
#[serde(deny_unknown_fields)]
pub struct Tls {
    /// The client certificate chain in PEM (template, usually a secret).
    pub certificate: String,
    /// Its private key in PEM (template, a secret).
    pub key: String,
}

/// One HTTP request. Every string is a template.
#[derive(Deserialize, Serialize, ToSchema, Clone, Debug)]
#[serde(deny_unknown_fields)]
pub struct Request {
    #[serde(default)]
    pub method: Method,
    /// The URL; its host must not change while paginating.
    pub url: String,
    /// Query parameters; a value rendering to `null` or "" is left out, a list repeats the parameter.
    #[serde(default)]
    #[schema(value_type = Value)]
    pub query: BTreeMap<String, Value>,
    #[serde(default)]
    pub headers: BTreeMap<String, String>,
    /// A JSON body (a tree of templates): GraphQL queries and variables, search bodies with paging fields.
    #[serde(default)]
    #[schema(value_type = Value)]
    pub body: Option<Value>,
    /// A form body (`application/x-www-form-urlencoded`).
    #[serde(default)]
    pub form: Option<BTreeMap<String, String>>,
    /// The answer's format.
    #[serde(default)]
    pub format: Format,
}

#[derive(Deserialize, Serialize, ToSchema, Clone, Copy, Debug, Default, PartialEq, Eq)]
#[serde(rename_all = "UPPERCASE")]
pub enum Method {
    #[default]
    Get,
    Post,
    Put,
    Patch,
}

#[derive(Deserialize, Serialize, ToSchema, Clone, Copy, Debug, Default, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Format {
    /// JSON (an empty answer is `null`).
    #[default]
    Json,
    /// One JSON value per line: `response` is their list.
    Ndjson,
    /// Prometheus / OpenMetrics text: `response` is a list of samples `{name, labels, value, type, help}`.
    Prometheus,
    /// Plain text: `response` is the text.
    Text,
    /// CSV with a header row: `response` is a list of objects by column name (EventLogFile downloads).
    Csv,
}

/// How a stream reads past its first page. Templates here read `response`, `headers` (lowercase names),
/// `records` (the page's), `page` and the request's variables.
#[derive(Deserialize, Serialize, ToSchema, Clone, Debug, Default)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Pagination {
    /// One page.
    #[default]
    None,
    /// A token from the answer (body or headers; any JSON value), sent back as `page.cursor`; stops when it is null
    /// or "".
    Cursor {
        /// Template.
        next: String,
    },
    /// The next page's URL from the answer (absolute or relative); stops when null or "".
    NextUrl {
        /// Template.
        next: String,
    },
    /// The `Link` header's `rel="next"` URL (GitHub, Okta, Shopify).
    LinkHeader,
    /// `page.offset` from `start`, `size` records a page (`page.size`); stops on a short page, or when the offset
    /// reaches `total`.
    Offset {
        size: u64,
        #[serde(default)]
        start: u64,
        /// Template: the number of records.
        #[serde(default)]
        total: Option<String>,
        /// The largest offset the API accepts: a time window reaching it is split in two and read again.
        #[serde(default)]
        max: Option<u64>,
    },
    /// `page.number` from `start` (default 1), `size` records a page; stops on a short page or after `total_pages`.
    Page {
        size: u64,
        #[serde(default = "one")]
        start: u64,
        /// Template.
        #[serde(default)]
        total_pages: Option<String>,
    },
    /// An asynchronous export: `create` starts it (`export.id` from `id`), `status` is polled on later runs until
    /// `ready`, then `chunk` reads each of `chunks` (`chunk` variable). The stream's `request` is not used.
    AsyncExport(Box<AsyncExport>),
}

fn one() -> u64 {
    1
}

#[derive(Deserialize, Serialize, ToSchema, Clone, Debug)]
#[serde(deny_unknown_fields)]
pub struct AsyncExport {
    pub create: Request,
    /// Template over the create answer.
    pub id: String,
    /// Polled with `export.id`.
    pub status: Request,
    /// Template over the status answer: true when the chunks can be read.
    pub ready: String,
    /// Template over the status answer: true when the export failed (it is started again).
    #[serde(default)]
    pub failed: Option<String>,
    /// Template over the status answer: the chunks' ids.
    pub chunks: String,
    /// Read with `export.id` and `chunk`.
    pub chunk: Request,
}

/// Something collected: one request (with pagination) per run, its records mapped to logs, metrics, events or
/// costs.
#[derive(Deserialize, Serialize, ToSchema, Clone, Debug)]
#[serde(deny_unknown_fields)]
pub struct Stream {
    /// `^[a-z][a-z0-9_]*$`: the job's scope.
    pub id: String,
    pub name: String,
    pub description: String,
    /// How often it runs. Duration, at least 1m, default 1m.
    #[serde(default)]
    pub interval: Option<String>,
    /// Whether a new connection collects it (admins can turn streams on and off).
    #[serde(default = "yes")]
    pub default_enabled: bool,
    /// A list (template over `config`, `discovery`, `tables`): the stream runs once per item, as `item`, each with
    /// its own cursor (domains, projects, tenants, metric groups).
    #[serde(default)]
    pub for_each: Option<String>,
    /// Replaces the manifest's `auth`.
    #[serde(default)]
    pub auth: Option<Auth>,
    /// Required unless `native` or an `async_export`.
    #[serde(default)]
    pub request: Option<Request>,
    #[serde(default)]
    pub pagination: Pagination,
    /// Template over `response`, `headers`: the page's records (a list; an object is one record). Default
    /// `{{ response }}`.
    #[serde(default)]
    pub records: Option<String>,
    /// Template over a record: its identity, for dedupe and logs' identity. Without one, the record's content is.
    #[serde(default)]
    pub record_id: Option<String>,
    /// Template over a record: records for which it is falsy are skipped.
    #[serde(default)]
    pub filter: Option<String>,
    /// Index-then-download: each record of the pages is an index entry (`parent`) whose `request` is read, and its
    /// records replace it (Salesforce EventLogFile CSVs, Microsoft 365 content blobs).
    #[serde(default)]
    pub expand: Option<Expand>,
    /// Values computed per record (templates over the record), readable as `vars.<name>` in the output. A var can't
    /// read another var.
    #[serde(default)]
    pub vars: BTreeMap<String, String>,
    /// Other endpoints read once per run (cached for `ttl`) to enrich records: `lookups.<name>[key]`.
    #[serde(default)]
    pub lookups: Vec<Lookup>,
    /// What a run reads again and what it skips.
    #[serde(default)]
    pub incremental: Incremental,
    /// Pages read per run (default 20); a time window not read to its end carries on next run.
    #[serde(default)]
    pub max_pages: Option<u32>,
    /// Replaces the manifest's rate limit.
    #[serde(default)]
    pub rate_limit: Option<RateLimit>,
    /// A request quota for this stream alone (a daily quota per log type).
    #[serde(default)]
    pub budget: Option<Budget>,
    /// Checked before the manifest's.
    #[serde(default)]
    pub errors: Vec<ErrorRule>,
    pub output: Output,
}

fn yes() -> bool {
    true
}

#[derive(Deserialize, Serialize, ToSchema, Clone, Debug)]
#[serde(deny_unknown_fields)]
pub struct Expand {
    /// Templates read `parent` (the index entry) and the stream's request variables.
    pub request: Request,
    /// As a stream's `records` (`parent` readable).
    #[serde(default)]
    pub records: Option<String>,
}

#[derive(Deserialize, Serialize, ToSchema, Clone, Debug)]
#[serde(deny_unknown_fields)]
pub struct Lookup {
    /// `lookups.<name>`.
    pub name: String,
    pub request: Request,
    #[serde(default)]
    pub pagination: Pagination,
    /// As a stream's `records`.
    #[serde(default)]
    pub records: Option<String>,
    /// Template over a lookup record: its key.
    pub key: String,
    /// Template over a lookup record: what is kept (default the record).
    #[serde(default)]
    pub value: Option<String>,
    /// Duration, default 1h.
    #[serde(default)]
    pub ttl: Option<String>,
    #[serde(default)]
    pub max_pages: Option<u32>,
}

#[derive(Deserialize, Serialize, ToSchema, Clone, Debug)]
#[serde(deny_unknown_fields)]
pub struct Discovery {
    /// `discovery.<name>`.
    pub name: String,
    pub request: Request,
    /// Template over `response`, `headers`: what is kept (default the answer).
    #[serde(default)]
    pub value: Option<String>,
    /// Duration, default 1h.
    #[serde(default)]
    pub ttl: Option<String>,
}

/// What a run reads again and what it skips.
#[derive(Deserialize, Serialize, ToSchema, Clone, Debug, Default)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Incremental {
    /// Everything, every run (state snapshots: metrics).
    #[default]
    None,
    /// Only records whose `record_id` wasn't sent before (lists without times: breached accounts, findings).
    NewRecords {
        /// Ids remembered (oldest forgotten first). Default 100000.
        #[serde(default)]
        max_seen: Option<u32>,
    },
    /// A time window from the cursor: `window.start` .. `window.end` (epoch ms) in request templates.
    Time(TimeCursor),
    /// An opaque checkpoint the API gives (`cursor.token` in request templates).
    Token {
        /// Template over `config`: the first run's token (default null).
        #[serde(default)]
        initial: Option<String>,
        /// Template over a page (`response`, `headers`, `records`, `last`): the token to keep; the last non-empty one
        /// is saved once the run's records are stored.
        next: String,
    },
}

#[derive(Deserialize, Serialize, ToSchema, Clone, Debug)]
#[serde(deny_unknown_fields)]
pub struct TimeCursor {
    /// Template over a record: its time (any form `epoch_ms` reads). Records outside the window are skipped.
    pub record_time: String,
    /// The order the API answers in: `newest_first` and `unordered` windows not read to their end carry on next run
    /// from where they stopped (the cursor moves only once a window is read); `oldest_first` ones move the cursor to
    /// the last record read.
    #[serde(default)]
    pub order: Order,
    /// How far back the first run reads. Duration, default 1d.
    #[serde(default)]
    pub lookback: Option<String>,
    /// How far before the cursor each window starts (late data); records already sent are skipped by id. Duration,
    /// default 0s.
    #[serde(default)]
    pub overlap: Option<String>,
    /// How recent a record must not be to be read (still being written). Duration, default 0s.
    #[serde(default)]
    pub settle: Option<String>,
    /// Splits the window into slices read oldest first (APIs limiting a request's range). Duration.
    #[serde(default)]
    pub slice: Option<String>,
    /// The largest window per run (catching up in steps). Duration.
    #[serde(default)]
    pub max_window: Option<String>,
    /// Where the cursor moves after a full window: its end, or the newest record's time.
    #[serde(default)]
    pub cursor: CursorValue,
}

#[derive(Deserialize, Serialize, ToSchema, Clone, Copy, Debug, Default, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Order {
    OldestFirst,
    NewestFirst,
    #[default]
    Unordered,
}

#[derive(Deserialize, Serialize, ToSchema, Clone, Copy, Debug, Default, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CursorValue {
    #[default]
    WindowEnd,
    LastRecord,
}

#[derive(Deserialize, Serialize, ToSchema, Clone, Debug, Default)]
#[serde(deny_unknown_fields)]
pub struct RateLimit {
    #[serde(default)]
    pub requests_per_minute: Option<u32>,
    /// The least time between two requests. Duration.
    #[serde(default)]
    pub min_interval: Option<String>,
}

#[derive(Deserialize, Serialize, ToSchema, Clone, Debug)]
#[serde(deny_unknown_fields)]
pub struct Budget {
    pub requests: u32,
    pub per: BudgetPeriod,
}

#[derive(Deserialize, Serialize, ToSchema, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum BudgetPeriod {
    Hour,
    Day,
    Month,
}

/// What a failed answer means. The first matching rule wins; without one, 401 is `auth`, 403 and 404
/// `access_denied`, 429 `throttled`, others `other`.
#[derive(Deserialize, Serialize, ToSchema, Clone, Debug)]
#[serde(deny_unknown_fields)]
pub struct ErrorRule {
    /// HTTP statuses it applies to (any when empty).
    #[serde(default)]
    pub status: Vec<u16>,
    /// Template over `response`, `status`: also required to match.
    #[serde(default)]
    pub when: Option<String>,
    pub kind: IssueKind,
    /// Template over `response`, `status`, `config`: what the page shows.
    pub message: String,
}

#[derive(Deserialize, Serialize, ToSchema, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum IssueKind {
    /// Credentials refused: retried at the stream's interval.
    Auth,
    /// Missing permission or plan: retried at the interval.
    AccessDenied,
    /// Rate limited: retried with exponential backoff.
    Throttled,
    /// Turned off on the vendor's side: retried daily.
    Disabled,
    Other,
}

/// The connection test: one request (or a stream's first page) and the message shown.
#[derive(Deserialize, Serialize, ToSchema, Clone, Debug)]
#[serde(deny_unknown_fields)]
pub struct Test {
    #[serde(default)]
    pub request: Option<Request>,
    /// A stream whose first page is read instead of `request` (`records` is then its records).
    #[serde(default)]
    pub stream: Option<String>,
    /// Template over `response`: whether it worked (default: the request succeeded).
    #[serde(default)]
    pub success: Option<String>,
    /// Template over `response`, `records`: the success message.
    pub message: String,
    /// Template: the message when `success` is false.
    #[serde(default)]
    pub failure: Option<String>,
    #[serde(default)]
    pub errors: Vec<ErrorRule>,
}

/// Where a stream's records go.
#[derive(Deserialize, Serialize, ToSchema, Clone, Debug, Default)]
#[serde(deny_unknown_fields)]
pub struct Output {
    #[serde(default)]
    pub logs: Option<LogsOut>,
    #[serde(default)]
    pub metrics: Vec<MetricOut>,
    #[serde(default)]
    pub events: Option<EventsOut>,
    /// Cloud Cost Management's costs (`otel.cloud_costs`), one row per record.
    #[serde(default)]
    pub costs: Option<CostsOut>,
}

/// A record as a log (Datadog's log shape: standard attributes such as `usr.*`, `network.client.ip`, `evt.name`,
/// `evt.outcome`). Templates read `record`, `vars`, `lookups`, `item`, `config`, `connection`, `tables`, `now`.
#[derive(Deserialize, Serialize, ToSchema, Clone, Debug)]
#[serde(deny_unknown_fields)]
pub struct LogsOut {
    /// Template.
    pub message: String,
    /// Template: `info` (default), `warn`, `error`, … (what Datadog's status remapper takes).
    #[serde(default)]
    pub status: Option<String>,
    /// Template, default the manifest's `source`.
    #[serde(default)]
    pub service: Option<String>,
    /// Template, default the manifest's `source`.
    #[serde(default)]
    pub source: Option<String>,
    /// Template, default `connection.instance`.
    #[serde(default)]
    pub host: Option<String>,
    /// Template: the log's time (default the record's time, else the run's).
    #[serde(default)]
    pub date: Option<String>,
    /// `ddtags` (templates; empty values are left out).
    #[serde(default)]
    pub tags: BTreeMap<String, String>,
    /// Attributes: a tree of templates; null, "" and empty objects are left out.
    #[serde(default)]
    #[schema(value_type = Value)]
    pub attributes: Option<Value>,
    /// Keeps the whole record under this attribute (`bitwarden`).
    #[serde(default)]
    pub keep_record_as: Option<String>,
}

/// One metric from each record (or from each entry of `each`). Templates read the record context and `entry`.
#[derive(Deserialize, Serialize, ToSchema, Clone, Debug)]
#[serde(deny_unknown_fields)]
pub struct MetricOut {
    /// Template (`netbird.peer.connected`, `openai.usage.{{ entry.key }}`).
    pub name: String,
    #[serde(rename = "type", default)]
    pub kind: MetricKind,
    /// Template: a number (records whose value is null are skipped).
    pub value: String,
    /// Template: a list, or an object whose entries are `{key, value}`: one point per entry, as `entry`.
    #[serde(default)]
    pub each: Option<String>,
    /// Tags (templates; empty values are left out).
    #[serde(default)]
    pub tags: BTreeMap<String, String>,
    /// Template: an object of more tags (Prometheus labels).
    #[serde(default)]
    pub tags_from: Option<String>,
    /// Template: the host the series is about (none by default: accounts aren't hosts).
    #[serde(default)]
    pub host: Option<String>,
    /// Template: the point's time (default the run's minute).
    #[serde(default)]
    pub time: Option<String>,
    /// Template over the record: points for which it is falsy are skipped.
    #[serde(default)]
    pub filter: Option<String>,
    /// Datadog unit name (`second`, `byte`), for the metric catalog and dashboards.
    #[serde(default)]
    pub unit: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
}

#[derive(Deserialize, Serialize, ToSchema, Clone, Copy, Debug, Default, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum MetricKind {
    #[default]
    Gauge,
    /// A delta over the run (a count of things since the last run).
    Count,
}

/// A record as an event (Event Management, `otel.otel_events`).
#[derive(Deserialize, Serialize, ToSchema, Clone, Debug)]
#[serde(deny_unknown_fields)]
pub struct EventsOut {
    /// Template.
    pub title: String,
    /// Template.
    pub text: String,
    /// Template: `error`, `warning`, `info` (default) or `success`.
    #[serde(default)]
    pub alert_type: Option<String>,
    /// Template: `normal` (default) or `low`.
    #[serde(default)]
    pub priority: Option<String>,
    #[serde(default)]
    pub host: Option<String>,
    #[serde(default)]
    pub tags: BTreeMap<String, String>,
    #[serde(default)]
    pub aggregation_key: Option<String>,
    /// Template: default the record's time.
    #[serde(default)]
    pub date: Option<String>,
}

/// A record as a cost (Cloud Cost Management).
#[derive(Deserialize, Serialize, ToSchema, Clone, Debug)]
#[serde(deny_unknown_fields)]
pub struct CostsOut {
    /// The provider column (`anthropic`, `databricks`).
    pub provider: String,
    /// Template: the day (or hour) the cost is for.
    pub date: String,
    /// Template: the amount.
    pub cost: String,
    /// Template, default `USD`.
    #[serde(default)]
    pub currency: Option<String>,
    /// Template: the service or product (`claude-opus`, `jobs-compute`).
    pub service: String,
    /// Template: the account, workspace or organization.
    #[serde(default)]
    pub account: Option<String>,
    /// Template: the usage type or SKU.
    #[serde(default)]
    pub usage_type: Option<String>,
    /// Template: the usage quantity.
    #[serde(default)]
    pub usage_amount: Option<String>,
    #[serde(default)]
    pub tags: BTreeMap<String, String>,
}

/// A one-off request (a setup step): its error rules and the message shown when it worked.
#[derive(Deserialize, Serialize, ToSchema, Clone, Debug)]
#[serde(deny_unknown_fields)]
pub struct Call {
    pub request: Request,
    /// Template over `response`: what the page shows.
    #[serde(default)]
    pub message: Option<String>,
    #[serde(default)]
    pub errors: Vec<ErrorRule>,
    /// Statuses that also count as done (409: the subscription exists).
    #[serde(default)]
    pub ok_status: Vec<u16>,
}

/// Where a push-only vendor sends, shown on the connector page.
#[derive(Deserialize, Serialize, ToSchema, Clone, Debug)]
#[serde(deny_unknown_fields)]
pub struct Push {
    pub kind: PushKind,
    /// Steps in the vendor's console (Markdown); `{{ intake }}` and `{{ source }}` are replaced.
    pub instructions: String,
    /// The URL to paste in the vendor's console (a template over `intake`, `source`, `config`; default by kind:
    /// `{{ intake }}/api/v2/logs?ddsource={{ source }}`).
    #[serde(default)]
    pub url: Option<String>,
    /// Headers the vendor must send (templates; `{{ api_key }}` is the org's ingest key placeholder).
    #[serde(default)]
    pub headers: BTreeMap<String, String>,
    /// How the vendor signs its requests, checked by the intake when set.
    #[serde(default)]
    pub signature: Option<PushSignature>,
    /// A request creating the push job or webhook on the vendor's side with the config entered (a Cloudflare Logpush
    /// job, a PagerDuty webhook subscription); `push.url` is readable as `push_url`.
    #[serde(default)]
    pub create: Option<Call>,
}

#[derive(Deserialize, Serialize, ToSchema, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PushKind {
    /// Datadog's logs API (`/api/v2/logs?ddsource=<source>`).
    Logs,
    /// OTLP/HTTP (`/v1/traces`, `/v1/metrics`, `/v1/logs`).
    Otlp,
    /// The intake's webhook endpoint (`/intake/webhook/<name>`), mapped by `webhook`.
    Webhook,
}

#[derive(Deserialize, Serialize, ToSchema, Clone, Debug)]
#[serde(deny_unknown_fields)]
pub struct PushSignature {
    /// The header carrying it (`x-vercel-signature`).
    pub header: String,
    pub algorithm: SignatureAlgorithm,
    /// The config field holding the shared secret.
    pub secret: String,
    /// Text before the value in the header, dropped before comparing (`sha256=` for GitHub's `X-Hub-Signature-256`,
    /// `Bearer ` for a token).
    #[serde(default)]
    pub prefix: Option<String>,
    /// Stripe: how old a signed timestamp may be (default `5m`).
    #[serde(default)]
    pub tolerance: Option<String>,
}

#[derive(Deserialize, Serialize, ToSchema, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SignatureAlgorithm {
    /// Hex HMAC-SHA1 of the body.
    HmacSha1,
    /// Hex HMAC-SHA256 of the body.
    HmacSha256,
    /// Stripe's `t=…,v1=…` (HMAC-SHA256 of `t.body`).
    Stripe,
    /// PagerDuty v3 `v1=…` (HMAC-SHA256 of the body; several when the secret is being rotated).
    PagerdutyV3,
    /// The header is the shared secret itself (a token the vendor sends as it is, after `prefix`).
    Token,
}

/// An inbound webhook (`/intake/webhook/<name>` on the intake), mapped like a stream's logs. `record` is each of
/// `records` (template over `body`, default the body).
#[derive(Deserialize, Serialize, ToSchema, Clone, Debug)]
#[serde(deny_unknown_fields)]
pub struct Webhook {
    /// The URL's last segment (default the id).
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub records: Option<String>,
    /// Template over `record`: the record's id (a log's identity: a delivery retried is stored once). Default: the
    /// record's JSON.
    #[serde(default)]
    pub record_id: Option<String>,
    /// Template over `record`: records for which it is falsy are dropped (pings, test deliveries).
    #[serde(default)]
    pub filter: Option<String>,
    /// Each record as a log.
    #[serde(default)]
    pub logs: Option<LogsOut>,
    /// Each record as an event (Event Management).
    #[serde(default)]
    pub events: Option<EventsOut>,
}

#[derive(Deserialize, Serialize, ToSchema, Clone, Debug, Default)]
#[serde(deny_unknown_fields)]
pub struct Assets {
    /// Dashboards in Datadog's JSON (`assets/dashboards/overview.json`), converted by `scripts/connector.mjs build`
    /// into `dist/dashboards.json`.
    #[serde(default)]
    pub dashboards: Vec<String>,
    /// A log pipeline in Datadog's pipeline JSON, installed when a connection is saved.
    #[serde(default)]
    pub pipeline: Option<String>,
    /// Recommended monitors in Datadog's monitor JSON.
    #[serde(default)]
    pub monitors: Vec<String>,
    /// Cloud SIEM detection rules in Datadog's rule JSON.
    #[serde(default)]
    pub security_rules: Vec<String>,
    /// Log-based metrics in Datadog's JSON (`datadog_logs_metric`), created when the integration is installed.
    #[serde(default)]
    pub log_metrics: Vec<String>,
    /// Metric metadata in integrations-core's `metadata.csv` format.
    #[serde(default)]
    pub metadata: Option<String>,
}

/// Recorded answers (fictional data: `example.com`, the e-commerce demo names) for the replay tests and mock mode.
#[derive(Deserialize, Serialize, ToSchema, Clone, Debug)]
#[serde(deny_unknown_fields)]
pub struct Mock {
    /// The connection's configuration.
    #[serde(default)]
    #[schema(value_type = Value)]
    pub config: BTreeMap<String, Value>,
    /// Its secrets (fake).
    #[serde(default)]
    pub secrets: BTreeMap<String, String>,
    /// The run's time (RFC 3339).
    pub now: String,
    pub responses: Vec<MockResponse>,
    /// What each stream (by id) and `test` must give.
    #[serde(default)]
    pub expect: BTreeMap<String, Expectation>,
}

/// A recorded answer, for the requests matching it (first match; the last match repeats).
#[derive(Deserialize, Serialize, ToSchema, Clone, Debug)]
#[serde(deny_unknown_fields)]
pub struct MockResponse {
    #[serde(default)]
    pub method: Method,
    /// The URL without its query.
    pub url: String,
    /// Query parameters the request must have (others are ignored).
    #[serde(default)]
    pub query: BTreeMap<String, String>,
    /// Text the request body must contain.
    #[serde(default)]
    pub body_contains: Option<String>,
    #[serde(default = "ok")]
    pub status: u16,
    #[serde(default)]
    pub headers: BTreeMap<String, String>,
    /// The answer inline.
    #[serde(default)]
    #[schema(value_type = Value)]
    pub body: Option<Value>,
    /// The answer from a file of the package (`fixtures/events.json`).
    #[serde(default)]
    pub file: Option<String>,
}

fn ok() -> u16 {
    200
}

#[derive(Deserialize, Serialize, ToSchema, Clone, Debug, Default)]
#[serde(deny_unknown_fields)]
pub struct Expectation {
    #[serde(default)]
    pub records: Option<usize>,
    #[serde(default)]
    pub logs: Option<usize>,
    /// Points.
    #[serde(default)]
    pub metrics: Option<usize>,
    #[serde(default)]
    pub events: Option<usize>,
    #[serde(default)]
    pub costs: Option<usize>,
    /// A file of the package with the expected output (`OC_CONNECTOR_UPDATE=1` rewrites it).
    #[serde(default)]
    pub golden: Option<String>,
    /// Records a second run right after reads (with the first run's state): what incremental reading skips.
    #[serde(default)]
    pub rerun: Option<usize>,
    /// For `test`: whether it succeeds.
    #[serde(default)]
    pub ok: Option<bool>,
    /// For `test`: text its message contains.
    #[serde(default)]
    pub message: Option<String>,
}

/// A duration as manifests write it: a number and `s`, `m`, `h` or `d`.
pub fn duration_ms(s: &str) -> Result<i64, String> {
    let s = s.trim();
    let (n, unit) = s.split_at(s.find(|c: char| !c.is_ascii_digit()).unwrap_or(s.len()));
    let n: i64 = n.parse().map_err(|_| format!("{s:?} is not a duration (30s, 5m, 2h, 1d)"))?;
    let per = match unit {
        "ms" => 1,
        "s" => 1000,
        "m" => 60_000,
        "h" => 3_600_000,
        "d" => 86_400_000,
        _ => return Err(format!("{s:?} is not a duration (30s, 5m, 2h, 1d)")),
    };
    Ok(n * per)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn durations() {
        assert_eq!(duration_ms("90s"), Ok(90_000));
        assert_eq!(duration_ms("1d"), Ok(86_400_000));
        assert_eq!(duration_ms("250ms"), Ok(250));
        assert!(duration_ms("1w").is_err());
        assert!(duration_ms("m").is_err());
    }

    #[test]
    fn unknown_fields_and_kinds_are_refused() {
        let auth: Result<Auth, _> = serde_yaml_ng::from_str("kind: bearer\ntoken: x\nextra: 1");
        assert!(auth.unwrap_err().to_string().contains("extra"));
        let auth: Result<Auth, _> = serde_yaml_ng::from_str("kind: magic");
        assert!(auth.unwrap_err().to_string().contains("magic"));
        let auth: Auth = serde_yaml_ng::from_str("kind: api_key\nname: hibp-api-key\nvalue: '{{ secrets.api_key }}'").unwrap();
        assert!(matches!(auth, Auth::ApiKey { location: KeyIn::Header, .. }));
        let p: Pagination = serde_yaml_ng::from_str("kind: offset\nsize: 100\nmax: 10000").unwrap();
        assert!(matches!(p, Pagination::Offset { size: 100, start: 0, max: Some(10000), .. }));
        let p: Pagination = serde_yaml_ng::from_str("kind: link_header").unwrap();
        assert!(matches!(p, Pagination::LinkHeader));
    }
}
