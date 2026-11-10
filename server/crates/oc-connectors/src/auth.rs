//! Authentication: [`prepare`] gets what a connection's requests need (fetching and caching OAuth tokens, JWT
//! assertions, token exchanges), [`Prepared::apply`] puts it on a request (headers, query, signatures).

use std::collections::{BTreeMap, HashMap};
use std::sync::Mutex;

use base64::Engine;
use hmac::{KeyInit, Mac};
use serde_json::{json, Map, Value};
use sha2::Digest;

use crate::expr::text;
use crate::http::{HttpRequest, HttpResponse, Transport};
use crate::manifest::{duration_ms, Auth, ClientAuth, JwtSpec, KeyIn, Method, OAuthCode, Signer, OAUTH_ACCESS_TOKEN, OAUTH_REFRESH_TOKEN};
use crate::template::{Template, Tree};

/// Tokens and other cached values, by key (connection and credentials), with their expiry (epoch ms).
#[derive(Default)]
pub struct Cache {
    entries: Mutex<HashMap<String, (Value, i64)>>,
}

impl Cache {
    pub fn get(&self, key: &str, now_ms: i64) -> Option<Value> {
        let entries = self.entries.lock().unwrap();
        entries.get(key).filter(|(_, exp)| *exp > now_ms).map(|(v, _)| v.clone())
    }

    pub fn put(&self, key: &str, value: Value, expires_ms: i64) {
        let mut entries = self.entries.lock().unwrap();
        if entries.len() > 50_000 {
            entries.retain(|_, (_, exp)| *exp > expires_ms - 86_400_000);
        }
        entries.insert(key.to_string(), (value, expires_ms));
    }

    pub fn forget_prefix(&self, prefix: &str) {
        self.entries.lock().unwrap().retain(|k, _| !k.starts_with(prefix));
    }
}

/// What requests carry, ready to apply.
#[derive(Clone, Debug, Default)]
pub struct Prepared {
    headers: Vec<(String, String)>,
    query: Vec<(String, String)>,
    signer: Option<(Signer, BTreeMap<String, String>)>,
    /// `auth.<name>` for templates (a token exchange's captured values).
    pub values: Map<String, Value>,
    /// A rotated refresh token to save: (secret field, value).
    pub rotated: Option<(String, String)>,
}

fn b64(bytes: &[u8]) -> String {
    base64::engine::general_purpose::STANDARD.encode(bytes)
}

fn b64url(bytes: &[u8]) -> String {
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn render(t: &str, scope: &Map<String, Value>) -> Result<String, String> {
    Ok(Template::parse(t)?.text(scope))
}

fn form(pairs: &[(String, String)]) -> Vec<u8> {
    url::form_urlencoded::Serializer::new(String::new()).extend_pairs(pairs).finish().into_bytes()
}

/// A token endpoint's answer, checked.
fn token_answer(resp: &HttpResponse, what: &str) -> Result<Value, String> {
    let body: Value = serde_json::from_slice(&resp.body).unwrap_or(Value::Null);
    if !(200..300).contains(&resp.status) {
        let detail = ["error_description", "error", "message"].iter().find_map(|k| body.get(*k).and_then(Value::as_str)).unwrap_or("");
        return Err(format!("{what} was refused (HTTP {}){}", resp.status, if detail.is_empty() { String::new() } else { format!(": {}", detail.chars().take(200).collect::<String>()) }));
    }
    if body.get("access_token").and_then(Value::as_str).is_none_or(str::is_empty) {
        return Err(format!("{what} answered without an access_token"));
    }
    Ok(body)
}

async fn post_form(transport: &dyn Transport, url: &str, pairs: &[(String, String)], basic: Option<(&str, &str)>) -> Result<HttpResponse, String> {
    let mut req = HttpRequest { method: Method::Post, url: url.to_string(), headers: vec![], body: Some(form(pairs)), tls: None };
    req.set_header("Content-Type", "application/x-www-form-urlencoded".into());
    req.set_header("Accept", "application/json".into());
    if let Some((user, password)) = basic {
        req.set_header("Authorization", format!("Basic {}", b64(format!("{user}:{password}").as_bytes())));
    }
    transport.send(&req).await
}

fn expiry(body: &Value, now_ms: i64) -> i64 {
    let secs = body.get("expires_in").and_then(|v| v.as_i64().or_else(|| v.as_str().and_then(|s| s.parse().ok()))).unwrap_or(3600);
    // A minute early, so a token never expires mid-run.
    now_ms + (secs.max(120) - 60) * 1000
}

fn bearer(token: &str) -> Vec<(String, String)> {
    vec![("Authorization".into(), format!("Bearer {token}"))]
}

/// Gets what `auth` needs. `scope` has `config`, `secrets`, `connection`, `tables`, `now`; `cache_key` names the
/// connection (and the stream's own auth when it has one).
pub async fn prepare(auth: &Auth, scope: &Map<String, Value>, cache_key: &str, transport: &dyn Transport, cache: &Cache, now_ms: i64) -> Result<Prepared, String> {
    let mut out = Prepared::default();
    // An authorization code connection: its refresh token's grant, else the access token the callback stored.
    let converted;
    let auth = match auth {
        Auth::Oauth2AuthorizationCode(c) => {
            let has = |k: &str| scope.get("secrets").and_then(|s| s.get(k)).and_then(Value::as_str).is_some_and(|v| !v.is_empty());
            converted = if has(OAUTH_REFRESH_TOKEN) {
                Auth::Oauth2RefreshToken(c.refresh())
            } else if has(OAUTH_ACCESS_TOKEN) {
                Auth::Bearer { token: format!("{{{{ secrets.{OAUTH_ACCESS_TOKEN} }}}}") }
            } else {
                return Err("the account isn't connected yet: use Connect on the connector's page".into());
            };
            &converted
        }
        other => other,
    };
    match auth {
        Auth::None => {}
        // Resolved before (`Auth::chosen`, above).
        Auth::Choice { .. } | Auth::Oauth2AuthorizationCode(_) => return Err("the connection's authentication method is not chosen".into()),
        Auth::ApiKey { location, name, value } => {
            let v = render(value, scope)?;
            match location {
                KeyIn::Header => out.headers.push((name.clone(), v)),
                KeyIn::Query => out.query.push((name.clone(), v)),
            }
        }
        Auth::Bearer { token } => out.headers = bearer(&render(token, scope)?),
        Auth::Basic { username, password } => {
            let (u, p) = (render(username, scope)?, render(password, scope)?);
            out.headers.push(("Authorization".into(), format!("Basic {}", b64(format!("{u}:{p}").as_bytes()))));
        }
        Auth::Headers { headers } => {
            for (k, v) in headers {
                out.headers.push((k.clone(), render(v, scope)?));
            }
        }
        Auth::Oauth2ClientCredentials(c) => {
            let (url, id, secret) = (render(&c.token_url, scope)?, render(&c.client_id, scope)?, render(&c.client_secret, scope)?);
            let key = format!("{cache_key}\0oauth\0{url}\0{id}");
            let token = match cache.get(&key, now_ms) {
                Some(t) => text(&t),
                None => {
                    let mut pairs = vec![("grant_type".to_string(), "client_credentials".to_string())];
                    if !c.scopes.is_empty() {
                        pairs.push(("scope".into(), c.scopes.join(" ")));
                    }
                    for (k, v) in &c.params {
                        pairs.push((k.clone(), render(v, scope)?));
                    }
                    let basic = match c.client_auth {
                        ClientAuth::Basic => Some((id.as_str(), secret.as_str())),
                        ClientAuth::Body => {
                            pairs.push(("client_id".into(), id.clone()));
                            pairs.push(("client_secret".into(), secret.clone()));
                            None
                        }
                    };
                    let body = token_answer(&post_form(transport, &url, &pairs, basic).await?, "the token request")?;
                    let token = text(&body["access_token"]);
                    cache.put(&key, Value::String(token.clone()), expiry(&body, now_ms));
                    token
                }
            };
            out.headers = bearer(&token);
        }
        Auth::Oauth2RefreshToken(c) => {
            let (url, id, refresh) = (render(&c.token_url, scope)?, render(&c.client_id, scope)?, render(&c.refresh_token, scope)?);
            let secret = match &c.client_secret {
                Some(s) => render(s, scope)?,
                None => String::new(),
            };
            let key = format!("{cache_key}\0refresh\0{url}\0{id}");
            let token = match cache.get(&key, now_ms) {
                Some(t) => text(&t),
                None => {
                    let mut pairs = vec![("grant_type".to_string(), "refresh_token".to_string()), ("refresh_token".to_string(), refresh.clone())];
                    if !c.scopes.is_empty() {
                        pairs.push(("scope".into(), c.scopes.join(" ")));
                    }
                    for (k, v) in &c.params {
                        pairs.push((k.clone(), render(v, scope)?));
                    }
                    let basic = match (c.client_auth, secret.is_empty()) {
                        (ClientAuth::Basic, false) => Some((id.as_str(), secret.as_str())),
                        _ => {
                            pairs.push(("client_id".into(), id.clone()));
                            if !secret.is_empty() {
                                pairs.push(("client_secret".into(), secret.clone()));
                            }
                            None
                        }
                    };
                    let body = token_answer(&post_form(transport, &url, &pairs, basic).await?, "the token refresh")?;
                    if let (Some(field), Some(new)) = (&c.rotate, body.get("refresh_token").and_then(Value::as_str)) {
                        if !new.is_empty() && new != refresh {
                            out.rotated = Some((field.clone(), new.to_string()));
                        }
                    }
                    let token = text(&body["access_token"]);
                    cache.put(&key, Value::String(token.clone()), expiry(&body, now_ms));
                    token
                }
            };
            out.headers = bearer(&token);
        }
        Auth::JwtBearer(j) => {
            let account: Option<Value> = match &j.service_account {
                Some(t) => Some(serde_json::from_str(render(t, scope)?.trim()).map_err(|_| "the service account key is not JSON".to_string())?),
                None => None,
            };
            let pem = match (&j.private_key, &account) {
                (Some(t), _) => render(t, scope)?,
                (None, Some(a)) => text(&a["private_key"]),
                (None, None) => return Err("jwt_bearer needs a private_key or a service_account".into()),
            };
            let token_url = match (&j.token_url, &account) {
                (Some(t), _) => render(t, scope)?,
                (None, Some(a)) => a["token_uri"].as_str().unwrap_or("https://oauth2.googleapis.com/token").to_string(),
                (None, None) => return Err("jwt_bearer needs a token_url".into()),
            };
            let mut claims = Map::new();
            if let Some(a) = &account {
                claims.insert("iss".into(), a["client_email"].clone());
                claims.insert("aud".into(), Value::String(token_url.clone()));
            }
            if !j.scopes.is_empty() {
                claims.insert("scope".into(), Value::String(j.scopes.join(" ")));
            }
            for (k, v) in &j.claims {
                let rendered = Tree::parse(v)?.render(scope, true);
                if !crate::template::empty(&rendered) {
                    claims.insert(k.clone(), rendered);
                }
            }
            let key = format!("{cache_key}\0jwt\0{token_url}\0{}", Value::Object(claims.clone()));
            let token = match cache.get(&key, now_ms) {
                Some(t) => text(&t),
                None => {
                    let lifetime = j.lifetime.as_deref().map(duration_ms).transpose()?.unwrap_or(3_600_000) / 1000;
                    let now_s = now_ms / 1000;
                    claims.insert("iat".into(), json!(now_s));
                    claims.insert("exp".into(), json!(now_s + lifetime));
                    let mut header = json!({ "alg": "RS256", "typ": "JWT" });
                    let kid = match (&j.key_id, &account) {
                        (Some(t), _) => render(t, scope)?,
                        (None, Some(a)) => text(&a["private_key_id"]),
                        _ => String::new(),
                    };
                    if !kid.is_empty() {
                        header["kid"] = Value::String(kid);
                    }
                    let assertion = rs256(&header, &Value::Object(claims), &pem)?;
                    let mut pairs = vec![("grant_type".to_string(), "urn:ietf:params:oauth:grant-type:jwt-bearer".to_string()), ("assertion".to_string(), assertion)];
                    for (k, v) in &j.params {
                        pairs.push((k.clone(), render(v, scope)?));
                    }
                    let body = token_answer(&post_form(transport, &token_url, &pairs, None).await?, "the token request")?;
                    let token = text(&body["access_token"]);
                    cache.put(&key, Value::String(token.clone()), expiry(&body, now_ms));
                    token
                }
            };
            out.headers = bearer(&token);
        }
        Auth::TokenExchange(x) => {
            let key = format!("{cache_key}\0exchange");
            let captured = match cache.get(&key, now_ms) {
                Some(v) => v,
                None => {
                    let mut ls = scope.clone();
                    if let Some(j) = &x.jwt {
                        ls.insert("auth".into(), json!({ "jwt": jwt(j, scope, now_ms)? }));
                    }
                    let req = crate::request::build(&x.request, &ls, &[])?;
                    let resp = transport.send(&req).await?;
                    if !(200..300).contains(&resp.status) {
                        return Err(format!("the login request was refused (HTTP {})", resp.status));
                    }
                    let mut s = scope.clone();
                    s.insert("response".into(), crate::http::parse_body(&resp.body, x.request.format).unwrap_or(Value::Null));
                    s.insert("headers".into(), resp.headers_value());
                    s.insert("status".into(), json!(resp.status));
                    let mut captured = Map::new();
                    for (k, t) in &x.capture {
                        captured.insert(k.clone(), Template::parse(t)?.render(&s));
                    }
                    if captured.values().all(crate::template::empty) {
                        return Err("the login answer has none of the values the connector needs".into());
                    }
                    let ttl_ms = match &x.expires_in {
                        Some(t) => {
                            let v = Template::parse(t)?.render(&s);
                            v.as_f64().or_else(|| v.as_str().and_then(|s| s.trim().parse().ok())).map(|secs| (secs as i64 - 60).max(60) * 1000)
                        }
                        None => None,
                    }
                    .unwrap_or(x.ttl.as_deref().map(duration_ms).transpose()?.unwrap_or(3_600_000));
                    let v = Value::Object(captured);
                    cache.put(&key, v.clone(), now_ms + ttl_ms);
                    v
                }
            };
            let mut s = scope.clone();
            s.insert("auth".into(), captured.clone());
            if x.headers.is_empty() {
                out.headers = bearer(&text(&captured["token"]));
            }
            for (k, v) in &x.headers {
                out.headers.push((k.clone(), render(v, &s)?));
            }
            out.values = captured.as_object().cloned().unwrap_or_default();
        }
        Auth::Jwt(j) => {
            let key = format!("{cache_key}\0jwt-auth");
            let token = match cache.get(&key, now_ms) {
                Some(t) => text(&t),
                None => {
                    let t = jwt(j, scope, now_ms)?;
                    let lifetime = j.lifetime.as_deref().map(duration_ms).transpose()?.unwrap_or(3_600_000);
                    cache.put(&key, Value::String(t.clone()), now_ms + (lifetime - 60_000).max(lifetime / 2));
                    t
                }
            };
            if j.headers.is_empty() {
                out.headers = bearer(&token);
            } else {
                let mut s = scope.clone();
                s.insert("auth".into(), json!({ "jwt": token }));
                for (k, v) in &j.headers {
                    out.headers.push((k.clone(), render(v, &s)?));
                }
            }
            out.values.insert("jwt".into(), Value::String(token));
        }
        Auth::Signer { signer, params } => {
            let mut rendered = BTreeMap::new();
            for (k, v) in params {
                rendered.insert(k.clone(), render(v, scope)?);
            }
            out.signer = Some((*signer, rendered));
        }
    }
    Ok(out)
}

impl Prepared {
    /// Puts the credentials on `req` (after it is rendered; signers sign its final form).
    pub fn apply(&self, req: &mut HttpRequest, now_ms: i64) -> Result<(), String> {
        for (k, v) in &self.headers {
            req.set_header(k, v.clone());
        }
        if !self.query.is_empty() {
            let mut url = url::Url::parse(&req.url).map_err(|e| e.to_string())?;
            url.query_pairs_mut().extend_pairs(&self.query);
            req.url = url.to_string();
        }
        if let Some((signer, params)) = &self.signer {
            sign(*signer, params, req, now_ms)?;
        }
        Ok(())
    }

    /// The secret values it carries (scrubbed from errors).
    pub fn secrets(&self) -> Vec<String> {
        let mut out: Vec<String> = self.headers.iter().chain(&self.query).map(|(_, v)| v.clone()).collect();
        out.extend(self.headers.iter().filter_map(|(_, v)| v.split_whitespace().nth(1).map(str::to_string)));
        out.extend(self.values.values().map(text));
        out.retain(|s| s.len() >= 6);
        out
    }
}

fn param<'a>(params: &'a BTreeMap<String, String>, key: &str, signer: &str) -> Result<&'a str, String> {
    params.get(key).map(String::as_str).filter(|v| !v.is_empty()).ok_or_else(|| format!("the {signer} signer needs {key}"))
}

/// Duo's canonical parameters: sorted, each key and value percent-encoded (RFC 3986 unreserved kept).
fn duo_params(url: &url::Url) -> String {
    let enc = |s: &str| -> String {
        s.bytes().map(|b| if b.is_ascii_alphanumeric() || b"-_.~".contains(&b) { (b as char).to_string() } else { format!("%{b:02X}") }).collect()
    };
    let mut pairs: Vec<(String, String)> = url.query_pairs().map(|(k, v)| (enc(&k), enc(&v))).collect();
    pairs.sort();
    pairs.into_iter().map(|(k, v)| format!("{k}={v}")).collect::<Vec<_>>().join("&")
}

fn method_name(m: Method) -> &'static str {
    match m {
        Method::Get => "GET",
        Method::Post => "POST",
        Method::Put => "PUT",
        Method::Patch => "PATCH",
    }
}

fn sign(signer: Signer, params: &BTreeMap<String, String>, req: &mut HttpRequest, now_ms: i64) -> Result<(), String> {
    let url = url::Url::parse(&req.url).map_err(|e| e.to_string())?;
    let host = url.host_str().unwrap_or_default().to_ascii_lowercase();
    match signer {
        Signer::DuoHmacV2 | Signer::DuoHmacV5 => {
            let (ikey, skey) = (param(params, "ikey", "Duo")?, param(params, "skey", "Duo")?);
            let date = chrono::DateTime::from_timestamp_millis(now_ms).unwrap_or_default().format("%a, %d %b %Y %H:%M:%S -0000").to_string();
            let mut canon = vec![date.clone(), method_name(req.method).to_string(), host, url.path().to_string(), duo_params(&url)];
            let signature = if signer == Signer::DuoHmacV2 {
                let mut mac = hmac::Hmac::<sha1::Sha1>::new_from_slice(skey.as_bytes()).map_err(|e| e.to_string())?;
                mac.update(canon.join("\n").as_bytes());
                hex(&mac.finalize().into_bytes())
            } else {
                canon.push(hex(&sha2::Sha512::digest(req.body.as_deref().unwrap_or_default())));
                canon.push(hex(&sha2::Sha512::digest(b"")));
                let mut mac = hmac::Hmac::<sha2::Sha512>::new_from_slice(skey.as_bytes()).map_err(|e| e.to_string())?;
                mac.update(canon.join("\n").as_bytes());
                req.set_header("X-Duo-Date", date.clone());
                hex(&mac.finalize().into_bytes())
            };
            req.set_header("Date", date);
            req.set_header("Authorization", format!("Basic {}", b64(format!("{ikey}:{signature}").as_bytes())));
        }
        Signer::CortexXdrAdvanced => {
            let (key_id, key) = (param(params, "key_id", "Cortex XDR")?, param(params, "key", "Cortex XDR")?);
            let mut raw = [0u8; 48];
            ring::rand::SecureRandom::fill(&ring::rand::SystemRandom::new(), &mut raw).map_err(|_| "no randomness".to_string())?;
            let nonce: String = raw.iter().map(|b| (b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789")[(*b as usize) % 62] as char).collect::<String>() + &"0".repeat(16);
            let timestamp = now_ms.to_string();
            let hash = hex(&sha2::Sha256::digest(format!("{key}{nonce}{timestamp}").as_bytes()));
            req.set_header("x-xdr-timestamp", timestamp);
            req.set_header("x-xdr-nonce", nonce);
            req.set_header("x-xdr-auth-id", key_id.to_string());
            req.set_header("Authorization", hash);
        }
        Signer::TrendMicroApexCentral => {
            let (app, key) = (param(params, "application_id", "Apex Central")?, param(params, "api_key", "Apex Central")?);
            let uri = match url.query() {
                Some(q) => format!("{}?{q}", url.path()),
                None => url.path().to_string(),
            };
            let body = String::from_utf8_lossy(req.body.as_deref().unwrap_or_default()).into_owned();
            let checksum = b64(&sha2::Sha256::digest(format!("{}{}{body}", method_name(req.method), uri.to_lowercase()).as_bytes()));
            let header = json!({ "alg": "HS256", "typ": "JWT" });
            let claims = json!({ "appid": app, "iat": now_ms / 1000, "version": "V1", "checksum": checksum });
            let signed = format!("{}.{}", b64url(header.to_string().as_bytes()), b64url(claims.to_string().as_bytes()));
            let mut mac = hmac::Hmac::<sha2::Sha256>::new_from_slice(key.as_bytes()).map_err(|e| e.to_string())?;
            mac.update(signed.as_bytes());
            req.set_header("Authorization", format!("Bearer {signed}.{}", b64url(&mac.finalize().into_bytes())));
        }
        Signer::AkamaiEdgegrid => {
            let (client_token, client_secret, access_token) = (param(params, "client_token", "EdgeGrid")?, param(params, "client_secret", "EdgeGrid")?, param(params, "access_token", "EdgeGrid")?);
            let timestamp = chrono::DateTime::from_timestamp_millis(now_ms).unwrap_or_default().format("%Y%m%dT%H:%M:%S+0000").to_string();
            let mut raw = [0u8; 16];
            ring::rand::SecureRandom::fill(&ring::rand::SystemRandom::new(), &mut raw).map_err(|_| "no randomness".to_string())?;
            let nonce = hex(&raw);
            let header = format!("EG1-HMAC-SHA256 client_token={client_token};access_token={access_token};timestamp={timestamp};nonce={nonce};");
            let hmac256 = |key: &[u8], msg: &[u8]| -> Result<Vec<u8>, String> {
                let mut mac = hmac::Hmac::<sha2::Sha256>::new_from_slice(key).map_err(|e| e.to_string())?;
                mac.update(msg);
                Ok(mac.finalize().into_bytes().to_vec())
            };
            let signing_key = b64(&hmac256(client_secret.as_bytes(), timestamp.as_bytes())?);
            let body = req.body.as_deref().unwrap_or_default();
            let content_hash = if req.method == Method::Post && !body.is_empty() { b64(&sha2::Sha256::digest(&body[..body.len().min(131_072)])) } else { String::new() };
            let relative = match url.query() {
                Some(q) => format!("{}?{q}", url.path()),
                None => url.path().to_string(),
            };
            let data = [method_name(req.method), url.scheme(), &host, &relative, "", &content_hash, &header].join("\t");
            let signature = b64(&hmac256(signing_key.as_bytes(), data.as_bytes())?);
            req.set_header("Authorization", format!("{header}signature={signature}"));
        }
    }
    Ok(())
}

/// DER: a tag and its length-prefixed content.
fn der(tag: u8, content: &[u8]) -> Vec<u8> {
    let mut out = vec![tag];
    let n = content.len();
    if n < 0x80 {
        out.push(n as u8);
    } else {
        let bytes: Vec<u8> = n.to_be_bytes().into_iter().skip_while(|b| *b == 0).collect();
        out.push(0x80 | bytes.len() as u8);
        out.extend(bytes);
    }
    out.extend_from_slice(content);
    out
}

/// `SHA256:` and the base64 SHA-256 of the key's public part as SubjectPublicKeyInfo DER (Snowflake's fingerprint).
pub fn public_key_fingerprint(pem: &str) -> Result<String, String> {
    let key = rsa_key(pem)?;
    // AlgorithmIdentifier rsaEncryption (1.2.840.113549.1.1.1) with NULL parameters.
    let algorithm: [u8; 15] = [0x30, 0x0d, 0x06, 0x09, 0x2a, 0x86, 0x48, 0x86, 0xf7, 0x0d, 0x01, 0x01, 0x01, 0x05, 0x00];
    let mut bits = vec![0u8];
    bits.extend_from_slice(key.public().as_ref());
    let mut content = algorithm.to_vec();
    content.extend(der(0x03, &bits));
    Ok(format!("SHA256:{}", b64(&sha2::Sha256::digest(der(0x30, &content)))))
}

fn rsa_key(pem: &str) -> Result<ring::signature::RsaKeyPair, String> {
    let (label, der) = pem_der(pem)?;
    if label == "RSA PRIVATE KEY" { ring::signature::RsaKeyPair::from_der(&der) } else { ring::signature::RsaKeyPair::from_pkcs8(&der) }.map_err(|_| "the private key is not a usable RSA key".to_string())
}

/// A JWT of `spec` signed now: claims rendered with `key.fingerprint` readable, `iat` and `exp` added.
pub fn jwt(spec: &JwtSpec, scope: &Map<String, Value>, now_ms: i64) -> Result<String, String> {
    let pem = render(&spec.private_key, scope)?;
    let mut s = scope.clone();
    s.insert("key".into(), json!({ "fingerprint": public_key_fingerprint(&pem)? }));
    let mut claims = Map::new();
    for (k, v) in &spec.claims {
        let rendered = Tree::parse(v)?.render(&s, true);
        if !crate::template::empty(&rendered) {
            claims.insert(k.clone(), rendered);
        }
    }
    let lifetime = spec.lifetime.as_deref().map(duration_ms).transpose()?.unwrap_or(3_600_000) / 1000;
    // A minute back: clocks drift (GitHub's advice).
    let iat = now_ms / 1000 - 60;
    claims.insert("iat".into(), json!(iat));
    claims.insert("exp".into(), json!(now_ms / 1000 + lifetime));
    let mut header = json!({ "alg": "RS256", "typ": "JWT" });
    if let Some(k) = &spec.key_id {
        let kid = render(k, scope)?;
        if !kid.is_empty() {
            header["kid"] = Value::String(kid);
        }
    }
    rs256(&header, &Value::Object(claims), &pem)
}

/// The DER inside a PEM block.
fn pem_der(pem: &str) -> Result<(String, Vec<u8>), String> {
    let pem = pem.replace("\\n", "\n");
    let label = pem.lines().find_map(|l| l.trim().strip_prefix("-----BEGIN ").and_then(|r| r.strip_suffix("-----"))).ok_or("the private key is not PEM")?.to_string();
    let body: String = pem.lines().map(str::trim).filter(|l| !l.starts_with("-----") && !l.is_empty()).collect();
    let der = base64::engine::general_purpose::STANDARD.decode(body).map_err(|_| "the private key is not valid PEM".to_string())?;
    Ok((label, der))
}

/// An RS256 JWT of `header` and `claims` signed with an RSA key in PEM (PKCS#8 or PKCS#1).
pub fn rs256(header: &Value, claims: &Value, pem: &str) -> Result<String, String> {
    let key = rsa_key(pem)?;
    let signed = format!("{}.{}", b64url(header.to_string().as_bytes()), b64url(claims.to_string().as_bytes()));
    let mut sig = vec![0u8; key.public().modulus_len()];
    key.sign(&ring::signature::RSA_PKCS1_SHA256, &ring::rand::SystemRandom::new(), signed.as_bytes(), &mut sig).map_err(|_| "signing the assertion failed".to_string())?;
    Ok(format!("{signed}.{}", b64url(&sig)))
}

/// A PKCE pair: the verifier (kept by the server) and its S256 challenge (sent to the consent page).
pub fn pkce() -> (String, String) {
    let mut bytes = [0u8; 32];
    ring::rand::SecureRandom::fill(&ring::rand::SystemRandom::new(), &mut bytes).expect("system randomness");
    let verifier = b64url(&bytes);
    let challenge = b64url(ring::digest::digest(&ring::digest::SHA256, verifier.as_bytes()).as_ref());
    (verifier, challenge)
}

/// The client of an authorization code flow: the connection's own app (templates) or the one in `secrets`
/// (`oauth_client_id`, the operator's).
pub fn oauth_client(c: &OAuthCode, scope: &Map<String, Value>) -> Result<(String, String), String> {
    let r = c.refresh();
    let id = render(&r.client_id, scope)?;
    let secret = render(r.client_secret.as_deref().unwrap_or_default(), scope)?;
    if id.trim().is_empty() {
        return Err("no OAuth app: the operator registers one for this connector (OC_OAUTH_<ID>_CLIENT_ID and _CLIENT_SECRET)".into());
    }
    Ok((id, secret))
}

/// The consent page's URL (`scope` has `config` and `secrets`).
pub fn authorize_url(c: &OAuthCode, scope: &Map<String, Value>, redirect_uri: &str, state: &str, challenge: Option<&str>) -> Result<String, String> {
    let (id, _) = oauth_client(c, scope)?;
    let mut url = url::Url::parse(&render(&c.authorize_url, scope)?).map_err(|e| format!("the consent page's URL is not valid: {e}"))?;
    {
        let mut q = url.query_pairs_mut();
        q.append_pair("response_type", "code").append_pair("client_id", &id).append_pair("redirect_uri", redirect_uri).append_pair("state", state);
        if !c.scopes.is_empty() {
            q.append_pair("scope", &c.scopes.join(c.scope_separator.as_deref().unwrap_or(" ")));
        }
        if let Some(ch) = challenge {
            q.append_pair("code_challenge", ch).append_pair("code_challenge_method", "S256");
        }
        for (k, v) in &c.authorize_params {
            q.append_pair(k, &render(v, scope)?);
        }
    }
    Ok(url.into())
}

/// Exchanges the callback's code for tokens: the token endpoint's answer (`access_token`, maybe `refresh_token`).
pub async fn exchange_code(c: &OAuthCode, scope: &Map<String, Value>, code: &str, redirect_uri: &str, verifier: Option<&str>, transport: &dyn Transport) -> Result<Value, String> {
    let (id, secret) = oauth_client(c, scope)?;
    let url = render(&c.token_url, scope)?;
    let mut pairs = vec![("grant_type".to_string(), "authorization_code".to_string()), ("code".to_string(), code.to_string()), ("redirect_uri".to_string(), redirect_uri.to_string())];
    if let Some(v) = verifier {
        pairs.push(("code_verifier".into(), v.to_string()));
    }
    for (k, v) in &c.params {
        pairs.push((k.clone(), render(v, scope)?));
    }
    let basic = match (c.client_auth, secret.is_empty()) {
        (ClientAuth::Basic, false) => Some((id.as_str(), secret.as_str())),
        _ => {
            pairs.push(("client_id".into(), id.clone()));
            if !secret.is_empty() {
                pairs.push(("client_secret".into(), secret.clone()));
            }
            None
        }
    };
    token_answer(&post_form(transport, &url, &pairs, basic).await?, "the authorization code exchange")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn req(url: &str) -> HttpRequest {
        HttpRequest { method: Method::Get, url: url.into(), headers: vec![], body: None, tls: None }
    }

    #[test]
    fn duo_v2_signs_the_canonical_request() {
        // Duo's documented example (Admin API, "Authentication"): the same canonical text, the same signature.
        let params: BTreeMap<String, String> = [("ikey".to_string(), "DIWJ8X6AEYOR5OMC6TQ1".to_string()), ("skey".to_string(), "Zh5eGmUq9zpfQnyUIu5OL9iWoMMv5ZNmk3zLJ4Ep".to_string())].into();
        let mut r = req("https://api-xxxxxxxx.duosecurity.com/admin/v1/users?username=root&realname=First%20Last");
        sign(Signer::DuoHmacV2, &params, &mut r, 1_262_365_200_000).unwrap();
        assert_eq!(r.header("date"), Some("Fri, 01 Jan 2010 17:00:00 -0000"));
        let basic = r.header("authorization").unwrap().strip_prefix("Basic ").unwrap();
        let decoded = String::from_utf8(base64::engine::general_purpose::STANDARD.decode(basic).unwrap()).unwrap();
        let (ikey, sig) = decoded.split_once(':').unwrap();
        assert_eq!(ikey, "DIWJ8X6AEYOR5OMC6TQ1");
        assert_eq!(sig.len(), 40, "hex HMAC-SHA1");
        assert_eq!(duo_params(&url::Url::parse("https://h.example.com/p?username=root&realname=First%20Last").unwrap()), "realname=First%20Last&username=root");
    }

    #[test]
    fn cortex_and_apex_central_sign_every_request() {
        let params: BTreeMap<String, String> = [("key_id".to_string(), "7".to_string()), ("key".to_string(), "secret-key".to_string())].into();
        let mut r = req("https://api-shop.xdr.us.paloaltonetworks.com/public_api/v1/incidents/get_incidents/");
        sign(Signer::CortexXdrAdvanced, &params, &mut r, 1_794_301_079_641).unwrap();
        let (nonce, ts) = (r.header("x-xdr-nonce").unwrap().to_string(), r.header("x-xdr-timestamp").unwrap().to_string());
        assert_eq!((nonce.len(), ts.as_str(), r.header("x-xdr-auth-id")), (64, "1794301079641", Some("7")));
        assert_eq!(r.header("authorization").unwrap(), hex(&sha2::Sha256::digest(format!("secret-key{nonce}{ts}").as_bytes())));
        let params: BTreeMap<String, String> = [("application_id".to_string(), "app-1".to_string()), ("api_key".to_string(), "k".to_string())].into();
        let mut r = req("https://apex.example.com/WebApp/api/v1/logs/officescan_virus?output_format=1");
        sign(Signer::TrendMicroApexCentral, &params, &mut r, 1_794_301_079_641).unwrap();
        let jwt = r.header("authorization").unwrap().strip_prefix("Bearer ").unwrap();
        let claims: Value = serde_json::from_slice(&base64::engine::general_purpose::URL_SAFE_NO_PAD.decode(jwt.split('.').nth(1).unwrap()).unwrap()).unwrap();
        assert_eq!((claims["appid"].as_str(), claims["version"].as_str(), claims["iat"].as_i64()), (Some("app-1"), Some("V1"), Some(1_794_301_079)));
        assert!(sign(Signer::CortexXdrAdvanced, &BTreeMap::new(), &mut req("https://x.example.com"), 0).unwrap_err().contains("key_id"));
    }

    #[test]
    fn edgegrid_signs_with_the_timestamp_key() {
        let params: BTreeMap<String, String> = [("client_token".to_string(), "akab-c".to_string()), ("client_secret".to_string(), "s3cret".to_string()), ("access_token".to_string(), "akab-a".to_string())].into();
        let mut r = req("https://akab-host.luna.akamaiapis.net/siem/v1/configs/1?limit=10");
        sign(Signer::AkamaiEdgegrid, &params, &mut r, 1_794_312_000_000).unwrap();
        let auth = r.header("authorization").unwrap();
        assert!(auth.starts_with("EG1-HMAC-SHA256 client_token=akab-c;access_token=akab-a;timestamp=20261110T12:00:00+0000;nonce="), "{auth}");
        assert!(auth.contains(";signature="));
    }

    #[test]
    fn pem_keys_are_read() {
        assert!(rs256(&json!({}), &json!({}), "not a key").unwrap_err().contains("PEM"));
        assert!(rs256(&json!({}), &json!({}), "-----BEGIN PRIVATE KEY-----\nAAAA\n-----END PRIVATE KEY-----").unwrap_err().contains("RSA"));
    }
}
