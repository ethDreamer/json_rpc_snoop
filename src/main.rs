use chrono;
use clap::{App, Arg};
use hyper::server::conn::AddrStream;
use hyper::service::{make_service_fn, service_fn};
use hyper::{Body, Client, Request, Response, Server};
use std::collections::HashSet;
use std::convert::Infallible;
use std::net::SocketAddr;
use std::str::FromStr;
use std::sync::Arc;
use termion::color;

mod utils;
use utils::{RpcErrorResponse, RpcRequest, SnoopError};

struct Inner {
    dest_uri: String,
    suppress_ok: Option<HashSet<String>>,
    suppress_all: Option<HashSet<String>>,
    //      (cyan,   red,    green,  reset)
    colors: (String, String, String, String),
}

#[derive(Clone)]
struct SnoopContext {
    inner: Arc<Inner>,
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

    let mut dest_request = Request::builder()
        .method(parts.method)
        .uri(&context.inner.dest_uri)
        .body(Body::from(request_bytes))?;

    for (key, value) in parts.headers.iter() {
        dest_request
            .headers_mut()
            .insert(key.clone(), value.clone());
    }

    Ok((dest_request, request_json))
}

async fn get_response(dest_request: Request<Body>) -> Result<(Response<Body>, String), SnoopError> {
    let dest_client = Client::new();
    let response = dest_client.request(dest_request).await?;

    let response_status = response.status();
    let response_version = response.version();
    let response_bytes = hyper::body::to_bytes(response.into_body()).await?;

    let response_json = {
        // Just return an error response if Utf8 conversion fails
        let json_str = std::str::from_utf8(&response_bytes)?;
        jsonxf::pretty_print(json_str).unwrap_or_else(|_| json_str.to_string())
    };

    let source_response = Response::builder()
        .status(response_status)
        .version(response_version)
        .body(Body::from(response_bytes))?;

    Ok((source_response, response_json))
}

fn print_request_response(request_json: String, response_json: String, context: &SnoopContext) {
    let (cyan, red, green, reset) = &context.inner.colors;
    let now = chrono::offset::Local::now()
        .format("%b %e %T%.3f %Y")
        .to_string();
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
                println!("{} REQUEST\n{}{}{}", now, cyan, request_json, reset);
                println!("{} RESPONSE\n{}{}{}", now, red, response_json, reset);
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
                println!("{} REQUEST\n{}{}{}", now, cyan, request_json, reset);
                println!("{} RESPONSE\n{}{}{}", now, green, response_json, reset);
            }
        }
        (Err(_), err_res) => {
            println!(
                "{} WARNING: request not formatted as JSON-RPC request:\n{}",
                now, request_json,
            );
            let color = match err_res {
                Ok(_) => red,
                Err(_) => green,
            };
            println!("{} RESPONSE\n{}{}{}", now, color, response_json, reset);
        }
    }
}

async fn handle_request(
    context: SnoopContext,
    _addr: SocketAddr,
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
            let (_, red, _, reset) = &context.inner.colors;
            println!("{}{}{}", red, error_body, reset);
            let source_response = Response::builder()
                .status(500)
                .body(Body::from(error_body))
                .unwrap();

            return Ok(source_response);
        }
    };

    let (source_response, response_json) = match get_response(dest_request).await {
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

    print_request_response(request_json, response_json, &context);

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
                .takes_value(true),
        )
        .arg(
            Arg::with_name("suppress-all")
                .short("S")
                .long("suppress-all")
                .help("Suppress success or error JSON RPC method calls of this type in output (can specify more than one)")
                .multiple(true)
                .takes_value(true),
        )
        .arg(
            Arg::with_name("RPC_ENDPOINT")
                .help("JSON-RPC endpoint to forward incoming requests to")
                .required(true)
                .index(1),
        )
        .get_matches();

    let color_tuple = if matches.is_present("no-color") {
        (String::new(), String::new(), String::new(), String::new())
    } else {
        (
            color::Fg(color::Cyan).to_string(),
            color::Fg(color::Red).to_string(),
            color::Fg(color::Green).to_string(),
            color::Fg(color::Reset).to_string(),
        )
    };

    let context = SnoopContext {
        inner: Arc::new(Inner {
            dest_uri: matches.value_of("RPC_ENDPOINT").unwrap().to_string(),
            colors: color_tuple,
            suppress_ok: matches
                .values_of("suppress-ok")
                .map(|values| values.into_iter().map(|s| s.to_string()).collect()),
            suppress_all: matches
                .values_of("suppress-all")
                .map(|values| values.into_iter().map(|s| s.to_string()).collect()),
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
