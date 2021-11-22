use clap::{App, Arg};
use colour::{cyan_ln, green_ln, red_ln};
use hyper::body::Bytes;
use hyper::server::conn::AddrStream;
use hyper::service::{make_service_fn, service_fn};
use hyper::{Body, Client, Request, Response, Server};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashSet;
use std::convert::Infallible;
use std::future::Future;
use std::net::SocketAddr;
use std::str::FromStr;
use std::sync::Arc;

mod utils;
use utils::SnoopError;

struct Inner {
    dest_uri: String,
    suppress: HashSet<String>,
}

#[derive(Clone)]
struct SnoopContext {
    inner: Arc<Inner>,
}

#[derive(Debug, Deserialize, Serialize)]
struct RpcRequest {
    id: u8,
    jsonrpc: String,
    method: String,
    params: Vec<Value>,
}
#[derive(Debug, Deserialize, Serialize)]
struct RpcError {
    code: i32,
    message: String,
}
#[derive(Debug, Deserialize, Serialize)]
struct RpcErrorResponse {
    id: u8,
    jsonrpc: String,
    error: RpcError,
}

async fn copy_request(
    source_request: Request<Body>,
    context: &SnoopContext,
) -> Result<(Request<Body>, Bytes), SnoopError> {
    let (parts, source_body) = source_request.into_parts();
    let source_bytes = hyper::body::to_bytes(source_body).await?;

    let mut dest_request = Request::builder()
        .method(parts.method)
        .uri(&context.inner.dest_uri)
        .body(Body::from(source_bytes.clone()))?;

    for (key, value) in parts.headers.iter() {
        dest_request
            .headers_mut()
            .insert(key.clone(), value.clone());
    }

    Ok((dest_request, source_bytes))
}

async fn get_response(dest_request: Request<Body>) -> Result<(Response<Body>, Bytes), SnoopError> {
    let dest_client = Client::new();
    let response = dest_client.request(dest_request).await?;

    let response_status = response.status();
    let response_version = response.version();
    let response_bytes = hyper::body::to_bytes(response.into_body()).await?;

    let source_response = Response::builder()
        .status(response_status)
        .version(response_version)
        .body(Body::from(response_bytes.clone()))?;

    Ok((source_response, response_bytes))
}

async fn handle_request(
    context: SnoopContext,
    _addr: SocketAddr,
    source_request: Request<Body>,
) -> Result<Response<Body>, SnoopError> {
    let (dest_request, source_bytes) = copy_request(source_request, &context).await?;

    // Just return an error response if Utf8 conversion fails
    let source_json = std::str::from_utf8(&source_bytes)?;

    let (source_response, response_bytes) = get_response(dest_request).await?;

    let response_json = match serde_json::from_str::<RpcRequest>(source_json) {
        Ok(rpc_request)
            if !context
                .inner
                .suppress
                .contains(&rpc_request.method.to_string()) =>
        {
            cyan_ln!("{}", jsonxf::pretty_print(source_json)?);
            Some(std::str::from_utf8(&response_bytes)?)
        }
        Err(_) => {
            println!(
                "WARNING: request not formatted as RPC request:\n{}",
                source_json
            );
            Some(std::str::from_utf8(&response_bytes)?)
        }
        _ => None,
    };

    if let Some(response_json) = response_json {
        if let Ok(_rpc_error) = serde_json::from_str::<RpcErrorResponse>(response_json) {
            red_ln!("{}", jsonxf::pretty_print(response_json)?);
        } else {
            green_ln!("{}", jsonxf::pretty_print(response_json)?);
        }
    }

    Ok(source_response)
}

#[tokio::main]
async fn main() {
    let matches = App::new("Json Snooping Tool")
        .version("0.1")
        .author("Mark Mackey <ethereumdreamer@gmail.com>")
        .about("Proxies an eth1 http json endpoint and dumps requests and responses to screen")
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
            Arg::with_name("suppress")
                .short("s")
                .long("suppress")
                .help("methods to suppress")
                .multiple(true)
                .takes_value(true),
        )
        .arg(
            Arg::with_name("ETH1_ENDPOINT")
                .help("Eth1 endpoint to forward incoming requests to")
                .required(true)
                .index(1),
        )
        .get_matches();

    let mut suppress = HashSet::new();
    if let Some(values) = matches.values_of("suppress") {
        for value in values {
            suppress.insert(value.to_string());
        }
    }

    let context = SnoopContext {
        inner: Arc::new(Inner {
            dest_uri: matches.value_of("ETH1_ENDPOINT").unwrap().to_string(),
            suppress,
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
        let service = service_fn(move |req| log_error(handle_request(context.clone(), addr, req)));

        // Return the service to hyper.
        async move { Ok::<_, Infallible>(service) }
    });

    // Run the server like above...
    match SocketAddr::from_str(&format!(
        "{}:{}",
        matches.value_of("bind-address").unwrap(),
        matches.value_of("port").unwrap()
    )) {
        Ok(socket) => {
            let server = Server::bind(&socket).serve(make_service);
            if let Err(e) = server.await {
                eprintln!("server error: {}", e);
            }
        }
        Err(e) => eprintln!("Error parsing listen address: {:?}", e),
    }
}

async fn log_error<F>(handle_func: F) -> Result<Response<Body>, Infallible>
where
    F: Future<Output = Result<Response<Body>, SnoopError>>,
{
    match handle_func.await {
        Ok(response) => Ok(response),
        Err(snoop_error) => {
            let error_body = serde_json::json!(RpcErrorResponse {
                id: 1,
                jsonrpc: "2.0".to_string(),
                error: RpcError {
                    code: -32603,
                    message: format!("{:?}", snoop_error)
                }
            })
            .to_string();
            red_ln!("{}", error_body);
            let source_response = Response::builder()
                .status(500)
                .body(Body::from(error_body))
                .unwrap();
            Ok(source_response)
        }
    }
}
