use chrono;
use clap::{App, Arg};
use hyper::http::header::{HeaderMap, HeaderName, HeaderValue};
use hyper::http::uri::Scheme;
use hyper::server::conn::AddrStream;
use hyper::service::{make_service_fn, service_fn};
use hyper::{Body, Client, Request, Response, Server, Uri};
use hyper_tls::HttpsConnector;
use std::collections::HashSet;
use std::convert::Infallible;
use std::net::SocketAddr;
use std::str::FromStr;
use std::sync::Arc;

mod utils;
use utils::{RpcErrorResponse, RpcRequest, SnoopError};
mod colors;
use colors::{color_treat, Colors};

struct Inner {
    dest_uri: Uri,
    suppress_ok: Option<HashSet<String>>,
    suppress_all: Option<HashSet<String>>,
    colors: Colors,
    log_headers: bool,
}

#[derive(Clone)]
struct SnoopContext {
    inner: Arc<Inner>,
}

fn remove_trailing_slashes(s: &str) -> &str {
    match s.char_indices().next_back() {
        Some((i, chr)) if chr == '/' => remove_trailing_slashes(&s[..i]),
        _ => s,
    }
}

fn get_hostport(uri: &Uri) -> HeaderValue {
    let mut hostport = String::new();
    if let Some(host) = uri.host() {
        hostport.push_str(host);
    }
    if let Some(port) = uri.port() {
        hostport.push_str(":");
        hostport.push_str(port.as_str());
    }

    HeaderValue::from_str(&hostport).expect("should be valid header")
}

async fn copy_request(
    source_request: Request<Body>,
    context: &SnoopContext,
) -> Result<(Request<Body>, String), SnoopError> {
    let (parts, request_body) = source_request.into_parts();
    let request_bytes = hyper::body::to_bytes(request_body).await?;

    let request_json = {
        // Just return an error response if Utf8 conversion fails
        let json_str = std::str::from_utf8(&request_bytes)?;
        jsonxf::pretty_print(json_str).unwrap_or_else(|_| json_str.to_string())
    };

    let mut dest_uri = remove_trailing_slashes(&context.inner.dest_uri.to_string()).to_string();
    dest_uri.push_str(parts.uri.path());
    let dest_uri = Uri::from_str(&dest_uri).unwrap_or_else(|_| context.inner.dest_uri.clone());

    let mut dest_request = Request::builder()
        .method(parts.method)
        .uri(&dest_uri)
        .body(Body::from(request_bytes))?;

    for (key, value) in parts.headers.iter() {
        let mut value = value.clone();
        if key.as_str().eq("accept-encoding") {
            // we don't want fancy encoding of the response
            continue;
        }
        if key.as_str().eq("host") {
            value = get_hostport(&context.inner.dest_uri)
        }
        dest_request.headers_mut().insert(key.clone(), value);
    }

    Ok((dest_request, request_json))
}

async fn get_response(
    dest_request: Request<Body>,
    context: &SnoopContext,
) -> Result<(Response<Body>, String), SnoopError> {
    let response = if context.inner.dest_uri.scheme() == Some(&Scheme::HTTPS) {
        let https = HttpsConnector::new();
        let dest_client = Client::builder().build::<_, hyper::Body>(https);
        dest_client.request(dest_request).await?
    } else {
        let dest_client = Client::new();
        dest_client.request(dest_request).await?
    };

    let (parts, response_body) = response.into_parts();
    let response_bytes = hyper::body::to_bytes(response_body).await?;

    let response_json = {
        // Just return an error response if Utf8 conversion fails
        let json_str = std::str::from_utf8(&response_bytes)?;
        jsonxf::pretty_print(json_str).unwrap_or_else(|_| json_str.to_string())
    };

    let mut source_response = Response::builder()
        .status(parts.status)
        .version(parts.version)
        .body(Body::from(response_bytes))?;

    for (key, value) in parts.headers.iter() {
        source_response
            .headers_mut()
            .insert(key.clone(), value.clone());
    }

    Ok((source_response, response_json))
}

fn print_request_response(
    request_json: String,
    response_json: String,
    request_headers: Vec<(HeaderName, HeaderValue)>,
    response_headers: Vec<(HeaderName, HeaderValue)>,
    context: &SnoopContext,
) {
    let now = chrono::offset::Local::now()
        .format("%b %e %T%.3f %Y")
        .to_string();

    let header_string =
        |headers: Vec<(HeaderName, HeaderValue)>, context: &SnoopContext| -> String {
            if !context.inner.log_headers || headers.is_empty() {
                String::new()
            } else {
                let mut result = String::from("headers:\n");
                for (key, value) in headers {
                    result.push_str(&format!("    ({},{:?})\n", key, value))
                }
                result
            }
        };

    match (
        serde_json::from_str::<RpcRequest>(&request_json),
        serde_json::from_str::<RpcErrorResponse>(&response_json),
    ) {
        (Ok(rpc_request), Ok(_)) => {
            if !context
                .inner
                .suppress_all
                .as_ref()
                .map(|all| all.contains(&rpc_request.method.to_string()))
                .unwrap_or(false)
            {
                println!(
                    "{} REQUEST\n{}{}",
                    now,
                    header_string(request_headers, context),
                    color_treat(request_json, context.inner.colors.cyan)
                );
                println!(
                    "{} RESPONSE\n{}{}",
                    now,
                    header_string(response_headers, context),
                    color_treat(response_json, context.inner.colors.red)
                );
            }
        }
        (Ok(rpc_request), Err(_)) => {
            if !context
                .inner
                .suppress_all
                .as_ref()
                .map(|all| all.contains(&rpc_request.method.to_string()))
                .unwrap_or(false)
                && !context
                    .inner
                    .suppress_ok
                    .as_ref()
                    .map(|ok| ok.contains(&rpc_request.method.to_string()))
                    .unwrap_or(false)
            {
                println!(
                    "{} REQUEST\n{}{}",
                    now,
                    header_string(request_headers, context),
                    color_treat(request_json, context.inner.colors.cyan)
                );
                println!(
                    "{} RESPONSE\n{}{}",
                    now,
                    header_string(response_headers, context),
                    color_treat(response_json, context.inner.colors.green)
                );
            }
        }
        (Err(e), err_res) => {
            println!(
                "{} WARNING: request not formatted as JSON-RPC request [{}]:\n{}{}",
                now,
                e,
                header_string(request_headers, context),
                color_treat(request_json, context.inner.colors.cyan),
            );
            let color = match err_res {
                Ok(_) => context.inner.colors.red,
                Err(_) => context.inner.colors.green,
            };
            println!(
                "{} RESPONSE\n{}{}",
                now,
                header_string(response_headers, context),
                color_treat(response_json, color)
            );
        }
    }
}

fn copy_headers(headers: &HeaderMap<HeaderValue>) -> Vec<(HeaderName, HeaderValue)> {
    headers
        .iter()
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect()
}

async fn handle_request(
    context: SnoopContext,
    _address: SocketAddr,
    source_request: Request<Body>,
) -> Result<Response<Body>, Infallible> {
    let (dest_request, request_json) = match copy_request(source_request, &context).await {
        Ok(result) => result,
        Err(e) => {
            let error_body = {
                let rpc_error = RpcErrorResponse::from(("Error processing request", e));
                serde_json::to_string_pretty(&rpc_error)
                    .unwrap_or_else(|_| serde_json::json!(rpc_error).to_string())
            };
            println!(
                "{}",
                color_treat(error_body.clone(), context.inner.colors.red)
            );
            let source_response = Response::builder()
                .status(500)
                .body(Body::from(error_body))
                .unwrap();

            return Ok(source_response);
        }
    };
    let request_headers = copy_headers(dest_request.headers());

    let (source_response, response_json) = match get_response(dest_request, &context).await {
        Ok(result) => result,
        Err(e) => {
            let error_body = {
                let rpc_error = RpcErrorResponse::from(("Error processing response", e));
                serde_json::to_string_pretty(&rpc_error)
                    .unwrap_or_else(|_| serde_json::json!(rpc_error).to_string())
            };
            let source_response = Response::builder()
                .status(500)
                .body(Body::from(error_body.clone()))
                .unwrap();
            (source_response, error_body)
        }
    };

    print_request_response(
        request_json,
        response_json,
        request_headers,
        copy_headers(source_response.headers()),
        &context,
    );

    Ok(source_response)
}

#[tokio::main]
async fn main() {
    let matches = App::new("JSON-RPC Snooping Tool")
        .version("0.1")
        .author("Mark Mackey <ethereumdreamer@gmail.com>")
        .about("Proxies an http JSON-RPC endpoint and dumps requests and responses to screen")
        .arg(
            Arg::with_name("bind-address")
                .short("b")
                .long("bind-address")
                .help("Address to bind to and listen for incoming requests")
                .required(false)
                .default_value("127.0.0.1")
                .takes_value(true),
        )
        .arg(
            Arg::with_name("port")
                .short("p")
                .long("port")
                .help("Port to listen for incoming requests")
                .required(false)
                .default_value("3000")
                .takes_value(true),
        )
        .arg(
            Arg::with_name("log-headers")
                .short("l")
                .long("log-headers")
                .help("Print the headers in addition to request/response")
                .required(false)
                .takes_value(false),
        )
        .arg(
            Arg::with_name("no-color")
                .long("no-color")
                .short("n")
                .required(false)
                .takes_value(false)
                .help("Do not use terminal colors in output"),
        )
        .arg(
            Arg::with_name("suppress-ok")
                .short("s")
                .long("suppress-ok")
                .help("Suppress successful JSON RPC method calls of this type in output (can specify more than one)")
                .multiple(true)
                .number_of_values(1)
                .takes_value(true),
        )
        .arg(
            Arg::with_name("suppress-all")
                .short("S")
                .long("suppress-all")
                .help("Suppress success or error JSON RPC method calls of this type in output (can specify more than one)")
                .multiple(true)
                .number_of_values(1)
                .takes_value(true),
        )
        .arg(
            Arg::with_name("RPC_ENDPOINT")
                .help("JSON-RPC endpoint to forward incoming requests")
                .required(true)
                .index(1),
        )
        .get_matches();

    let dest_uri: Uri = match matches.value_of("RPC_ENDPOINT").unwrap().parse() {
        Ok(uri) => uri,
        Err(e) => {
            eprintln!(
                "Unable to parse Uri from {}: {}",
                matches.value_of("RPC_ENDPOINT").unwrap(),
                e
            );
            return;
        }
    };

    let context = SnoopContext {
        inner: Arc::new(Inner {
            dest_uri,
            suppress_ok: matches
                .values_of("suppress-ok")
                .map(|values| values.into_iter().map(|s| s.to_string()).collect()),
            suppress_all: matches
                .values_of("suppress-all")
                .map(|values| values.into_iter().map(|s| s.to_string()).collect()),
            colors: Colors::new(matches.is_present("no-color")),
            log_headers: matches.is_present("log-headers"),
        }),
    };

    // A `MakeService` that produces a `Service` to handle each connection.
    let make_service = make_service_fn(move |conn: &AddrStream| {
        // We have to clone the context to share it with each invocation of
        // `make_service`. If your data doesn't implement `Clone` consider using
        // an `std::sync::Arc`.
        let context = context.clone();

        // You can grab the address of the incoming connection like so.
        let addr = conn.remote_addr();

        // Create a `Service` for responding to the request.
        let service = service_fn(move |req| handle_request(context.clone(), addr, req));

        // Return the service to hyper.
        async move { Ok::<_, Infallible>(service) }
    });

    match SocketAddr::from_str(&format!(
        "{}:{}",
        matches.value_of("bind-address").unwrap(),
        matches.value_of("port").unwrap()
    )) {
        Ok(socket) => match Server::try_bind(&socket) {
            Ok(server) => {
                if let Err(e) = server.serve(make_service).await {
                    eprintln!("server error: {}", e);
                }
            }
            Err(e) => {
                eprintln!("Unable to bind to socket: {}", e);
            }
        },
        Err(e) => eprintln!("Error parsing listen address: {:?}", e),
    }
}
