use std::{fs::File, io::BufReader, net::SocketAddr, sync::Arc};

use axum::{
    body::{to_bytes, Body},
    extract::{Path, State},
    http::{HeaderMap, HeaderName, HeaderValue, Request, Response, StatusCode, Uri},
    routing::{any, get},
    Router,
};
use dotenvy::dotenv;
use futures_util::StreamExt; // for .next() on streams
use http::header::{
    AUTHORIZATION, CONNECTION, HOST, PROXY_AUTHENTICATE, PROXY_AUTHORIZATION, TE, TRAILER,
    TRANSFER_ENCODING, UPGRADE, ACCESS_CONTROL_ALLOW_ORIGIN, ACCESS_CONTROL_ALLOW_METHODS,
    ACCESS_CONTROL_ALLOW_HEADERS, ACCESS_CONTROL_MAX_AGE, ACCESS_CONTROL_EXPOSE_HEADERS,
    ACCESS_CONTROL_ALLOW_CREDENTIALS, ACCESS_CONTROL_REQUEST_METHOD, ACCESS_CONTROL_REQUEST_HEADERS,
};
use rustls_pemfile::{certs, pkcs8_private_keys};
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;
use tower::ServiceBuilder;
use tower_http::{compression::CompressionLayer, trace::TraceLayer};
use tracing::{error, info};
use tracing_subscriber::{fmt::Subscriber, EnvFilter};

#[derive(Clone)]
struct AppState {
    client: reqwest::Client,
    service_token: Arc<String>,
    openai_api_key: Arc<String>,
}

#[tokio::main]
async fn main() {
    dotenv().ok();

    let filter =
        EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info,tower_http=info"));
    Subscriber::builder().with_env_filter(filter).init();

    let service_token = std::env::var("SERVICE_TOKEN").expect("SERVICE_TOKEN is required");
    let openai_api_key = std::env::var("OPENAI_API_KEY").expect("OPENAI_API_KEY is required");

    let client = reqwest::Client::builder()
        .pool_max_idle_per_host(8)
        .build()
        .expect("failed to build reqwest client");

    let state = AppState {
        client,
        service_token: Arc::new(service_token),
        openai_api_key: Arc::new(openai_api_key),
    };

    let app = Router::new()
        .route("/healthz", get(|| async { "ok" }))
        .route("/v1/*path", any(proxy_handler))
        .with_state(state)
        .layer(
            ServiceBuilder::new()
                .layer(CompressionLayer::new()) // gzip/br for non-SSE, harmless for SSE as we pass upstream headers
                .layer(TraceLayer::new_for_http()),
        );

    let host = std::env::var("BIND_HOST").unwrap_or_else(|_| "0.0.0.0".into());
    let port: u16 = std::env::var("BIND_PORT")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(8080);
    let addr: SocketAddr = format!("{host}:{port}")
        .parse()
        .expect("invalid bind addr");

    // Проверяем, нужно ли использовать HTTPS
    let cert_path = std::env::var("TLS_CERT_PATH").ok();
    let key_path = std::env::var("TLS_KEY_PATH").ok();

    if let (Some(cert_path), Some(key_path)) = (cert_path, key_path) {
        // HTTPS режим
        let certs = match load_certs(&cert_path) {
            Ok(certs) => certs,
            Err(e) => {
                error!("Failed to load certificates: {e}");
                return;
            }
        };
        let key = match load_private_key(&key_path) {
            Ok(key) => key,
            Err(e) => {
                error!("Failed to load private key: {e}");
                return;
            }
        };

        let config = match rustls::ServerConfig::builder()
            .with_safe_defaults()
            .with_no_client_auth()
            .with_single_cert(certs, key)
        {
            Ok(config) => config,
            Err(e) => {
                let err_msg = format!("Failed to create TLS config: {e}");
                error!("{}", err_msg);
                return;
            }
        };

        let tls_config = axum_server::tls_rustls::RustlsConfig::from_config(Arc::new(config));
        info!("listening on https://{addr}");
        axum_server::bind_rustls(addr, tls_config)
            .serve(app.into_make_service())
            .await
            .unwrap();
    } else {
        // HTTP режим
        let listener = tokio::net::TcpListener::bind(addr).await.unwrap();
        info!("listening on http://{addr}");
        axum::serve(listener, app).await.unwrap();
    }
}

async fn proxy_handler(
    State(state): State<AppState>,
    Path(tail): Path<String>,
    req: Request<Body>,
) -> Result<Response<Body>, (StatusCode, String)> {
    // Обработка OPTIONS запросов (preflight CORS)
    if req.method() == http::Method::OPTIONS {
        let mut response = Response::new(Body::empty());
        *response.status_mut() = StatusCode::NO_CONTENT;
        response.headers_mut().insert(
            ACCESS_CONTROL_ALLOW_ORIGIN,
            HeaderValue::from_static("*"),
        );
        response.headers_mut().insert(
            ACCESS_CONTROL_ALLOW_METHODS,
            HeaderValue::from_static("GET, POST, PUT, DELETE, OPTIONS, PATCH"),
        );
        response.headers_mut().insert(
            ACCESS_CONTROL_ALLOW_HEADERS,
            HeaderValue::from_static("Authorization, Content-Type, Accept, X-Requested-With"),
        );
        response.headers_mut().insert(
            ACCESS_CONTROL_MAX_AGE,
            HeaderValue::from_static("86400"),
        );
        return Ok(response);
    }

    // 1) Service token auth
    if !is_authorized(req.headers(), &state.service_token) {
        return Err((
            StatusCode::UNAUTHORIZED,
            "unauthorized: missing or invalid bearer token".into(),
        ));
    }

    // 2) Build OpenAI target URL (preserve query)
    let mut path_and_query = format!("/v1/{}", tail);
    if let Some(q) = req.uri().query() {
        path_and_query.push('?');
        path_and_query.push_str(q);
    }
    let target_uri: Uri = format!("https://api.openai.com{path_and_query}")
        .parse()
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("uri parse error: {e}")))?;

    // 3) Copy method/headers, replace Authorization with OpenAI key
    let (parts, body) = req.into_parts();

    // Если тело маленькое — читаем сразу (универсально для JSON/multipart). Для очень крупных тел
    // можно заменить на потоковую передачу через hyper, но для OpenAI этого обычно достаточно.
    let body_bytes = to_bytes(body, usize::MAX)
        .await
        .map_err(|e| (StatusCode::BAD_REQUEST, format!("read body error: {e}")))?;
    let reqwest_body = reqwest::Body::from(body_bytes.to_vec());

    let mut out_builder = state
        .client
        .request(parts.method, target_uri.to_string())
        .body(reqwest_body);

    let mut fwd_headers = HeaderMap::new();
    copy_headers_filtered(&parts.headers, &mut fwd_headers);
    fwd_headers.insert(
        AUTHORIZATION,
        HeaderValue::from_str(&format!("Bearer {}", state.openai_api_key))
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("hdr error: {e}")))?,
    );
    for (k, v) in fwd_headers.iter() {
        out_builder = out_builder.header(k, v);
    }

    // 4) Send upstream and stream response back byte-for-byte (SSE safe)
    let upstream = out_builder
        .send()
        .await
        .map_err(|e| (StatusCode::BAD_GATEWAY, format!("upstream send error: {e}")))?;

    let status = upstream.status();

    let mut resp_headers = HeaderMap::new();
    copy_response_headers_filtered(upstream.headers(), &mut resp_headers);

    // bytes_stream() -> forward as-is to client
    let mut upstream_stream = upstream.bytes_stream();

    let (tx, rx) = mpsc::channel::<Result<bytes::Bytes, std::io::Error>>(8);

    tokio::spawn(async move {
        while let Some(chunk_res) = upstream_stream.next().await {
            match chunk_res {
                Ok(chunk) => {
                    if tx.send(Ok(chunk)).await.is_err() {
                        break; // client disconnected
                    }
                }
                Err(e) => {
                    let _ = tx
                        .send(Err(std::io::Error::new(std::io::ErrorKind::Other, e)))
                        .await;
                    break;
                }
            }
        }
        // channel closes automatically when dropped
    });

    let stream = ReceiverStream::new(rx).map(|res| {
        res.map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))
    });
    let body = Body::from_stream(stream);

    let mut out = Response::new(body);
    *out.status_mut() = status;
    *out.headers_mut() = resp_headers;
    
    // Добавляем CORS заголовки к обычным ответам
    out.headers_mut().insert(
        ACCESS_CONTROL_ALLOW_ORIGIN,
        HeaderValue::from_static("*"),
    );
    out.headers_mut().insert(
        ACCESS_CONTROL_ALLOW_METHODS,
        HeaderValue::from_static("GET, POST, PUT, DELETE, OPTIONS, PATCH"),
    );
    out.headers_mut().insert(
        ACCESS_CONTROL_ALLOW_HEADERS,
        HeaderValue::from_static("Authorization, Content-Type, Accept, X-Requested-With"),
    );
    
    Ok(out)
}

fn is_authorized(headers: &HeaderMap, expected_token: &str) -> bool {
    match headers.get(AUTHORIZATION) {
        Some(v) => v
            .to_str()
            .ok()
            .and_then(|s| s.strip_prefix("Bearer "))
            .map(|t| t == expected_token)
            .unwrap_or(false),
        None => false,
    }
}

fn hop_by_hop_header(name: &HeaderName) -> bool {
    matches!(
        *name,
        CONNECTION
            | PROXY_AUTHENTICATE
            | PROXY_AUTHORIZATION
            | TE
            | TRAILER
            | TRANSFER_ENCODING
            | UPGRADE
            | HOST
    )
}

fn copy_headers_filtered(src: &HeaderMap, dst: &mut HeaderMap) {
    for (name, value) in src.iter() {
        if hop_by_hop_header(name) || name == &AUTHORIZATION {
            continue;
        }
        if let Ok(cloned) = HeaderValue::from_bytes(value.as_bytes()) {
            dst.insert(name.clone(), cloned);
        }
    }
}

fn is_cors_header(name: &HeaderName) -> bool {
    matches!(
        *name,
        ACCESS_CONTROL_ALLOW_ORIGIN
            | ACCESS_CONTROL_ALLOW_METHODS
            | ACCESS_CONTROL_ALLOW_HEADERS
            | ACCESS_CONTROL_MAX_AGE
            | ACCESS_CONTROL_EXPOSE_HEADERS
            | ACCESS_CONTROL_ALLOW_CREDENTIALS
            | ACCESS_CONTROL_REQUEST_METHOD
            | ACCESS_CONTROL_REQUEST_HEADERS
    )
}

fn copy_response_headers_filtered(src: &HeaderMap, dst: &mut HeaderMap) {
    for (name, value) in src.iter() {
        // Фильтруем hop-by-hop заголовки и CORS заголовки (они будут добавлены нами)
        if hop_by_hop_header(name) || is_cors_header(name) {
            continue;
        }
        if let Ok(cloned) = HeaderValue::from_bytes(value.as_bytes()) {
            dst.insert(name.clone(), cloned);
        }
    }
}

fn load_certs(path: &str) -> Result<Vec<rustls::Certificate>, String> {
    let cert_file = File::open(path)
        .map_err(|e| format!("Failed to open certificate file {path}: {e}"))?;
    let mut reader = BufReader::new(cert_file);
    let certs = certs(&mut reader)
        .map_err(|e| format!("Failed to parse certificate: {e}"))?;
    Ok(certs.into_iter().map(rustls::Certificate).collect())
}

fn load_private_key(path: &str) -> Result<rustls::PrivateKey, String> {
    let key_file = File::open(path)
        .map_err(|e| format!("Failed to open key file {path}: {e}"))?;
    let mut reader = BufReader::new(key_file);
    let keys = pkcs8_private_keys(&mut reader)
        .map_err(|e| format!("Failed to parse private key: {e}"))?;
    
    if keys.is_empty() {
        return Err("No private keys found in file".into());
    }
    
    Ok(rustls::PrivateKey(keys[0].clone()))
}
