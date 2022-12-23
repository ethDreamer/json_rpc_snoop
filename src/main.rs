use async_mutex::Mutex;
use chrono;
use clap::{App, Arg};
use hyper::http::header::{HeaderMap, HeaderName, HeaderValue};
use hyper::http::uri::Scheme;
use hyper::server::conn::AddrStream;
use hyper::service::{make_service_fn, service_fn};
use hyper::{Body, Client, Request, Response, Server, StatusCode, Uri};
use hyper_tls::HttpsConnector;
use rand::{Rng, SeedableRng};
use std::collections::HashMap;
use std::convert::Infallible;
use std::net::SocketAddr;
use std::str::FromStr;
use std::sync::Arc;
use tokio::time::{sleep, Duration};

mod utils;
use utils::{PacketType, RpcErrorResponse, RpcRequest, SnoopError, SuppressType};
mod colors;
use colors::{color_treat, Colors};

#[derive(Debug)]
struct Inner {
    dest_uri: Uri,
    rng: Mutex<rand::rngs::StdRng>,
    suppress_method: Option<HashMap<String, (i32, SuppressType)>>,
    suppress_path: Option<HashMap<String, (i32, SuppressType)>>,
    override_rpc: Option<Vec<String>>,
    colors: Colors,
    drop_request_rate: f32,
    drop_response_rate: f32,
    log_headers: bool,
}

#[derive(Clone, Debug)]
struct SnoopContext {
    inner: Arc<Inner>,
}

fn is_rpc_modules_request(request_json: &str) -> bool {
    serde_json::from_str::<RpcRequest>(request_json)
        .map(|rpc_request| rpc_request.method == "rpc_modules")
        .unwrap_or(false)
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

fn get_rpc_modules_override(rpc_modules: &Vec<String>) -> (Response<Body>, String) {
    let mut response_json = "{\n  \"jsonrpc\": \"2.0\",\n  \"result\": {\n".to_string();
    if let Some(module) = rpc_modules.first() {
        response_json.push_str(&format!("    \"{}\": \"1.0\"", module))
    }
    for module in rpc_modules.iter().skip(1) {
        response_json.push_str(",\n");
        response_json.push_str(&format!("    \"{}\": \"1.0\"", module));
    }
    response_json.push_str("\n  },\n  \"id\": 1\n}");

    let response = Response::builder()
        .status(200)
        .header("content-type", "application/json")
        .body(Body::from(response_json.clone()))
        .unwrap();

    (response, response_json)
}

async fn copy_request(
    source_request: Request<Body>,
    context: &SnoopContext,
) -> Result<(Request<Body>, String), SnoopError> {
    let (parts, request_body) = source_request.into_parts();
    let request_bytes = hyper::body::to_bytes(request_body).await?;

    let request_json = {
        if request_bytes.is_empty() {
            "null".to_string()
        } else {
            let json_str = std::str::from_utf8(&request_bytes)?;
            jsonxf::pretty_print(json_str).unwrap_or_else(|_| json_str.to_string())
        }
    };

    let construct_uri = !parts.uri.path().eq("/") || parts.uri.query().is_some();
    let mut dest_request = if construct_uri {
        let mut dest_uri =
            utils::remove_trailing_slashes(&context.inner.dest_uri.to_string()).to_string();
        dest_uri.push_str(parts.uri.path());
        if let Some(query) = parts.uri.query() {
            dest_uri.push_str("?");
            dest_uri.push_str(query);
        }
        let dest_uri =
            utils::parse_uri(&dest_uri).unwrap_or_else(|_| context.inner.dest_uri.clone());
        Request::builder()
            .method(parts.method)
            .uri(&dest_uri)
            .body(Body::from(request_bytes))?
    } else {
        Request::builder()
            .method(parts.method)
            .uri(&context.inner.dest_uri)
            .body(Body::from(request_bytes))?
    };

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
        if response_bytes.is_empty() {
            "null".to_string()
        } else {
            let json_str = std::str::from_utf8(&response_bytes)?;
            jsonxf::pretty_print(json_str).unwrap_or_else(|_| json_str.to_string())
        }
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

fn print_json(
    json: &str,
    headers: &Vec<(HeaderName, HeaderValue)>,
    json_type: PacketType,
    msg_info: &str,
    status: Option<StatusCode>,
    context: &SnoopContext,
) {
    let now = chrono::offset::Local::now()
        .format("%b %e %T%.3f %Y")
        .to_string();
    let header_string =
        |headers: &Vec<(HeaderName, HeaderValue)>, context: &SnoopContext| -> String {
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
    let msg_string = |msg: &str| -> String {
        if msg.is_empty() || msg.eq("/") {
            String::new()
        } else {
            msg.to_string()
        }
    };

    let color = match json_type {
        PacketType::Request => context.inner.colors.cyan,
        PacketType::RequestDropped(_) => context.inner.colors.white,
        PacketType::Response => match serde_json::from_str::<RpcErrorResponse>(json) {
            Ok(_) => context.inner.colors.red,
            Err(_) => context.inner.colors.green,
        },
        PacketType::ResponseDropped(_) => context.inner.colors.white,
    };

    let status_str = status
        .map(|s| format!(" (status {})", s))
        .unwrap_or_else(String::new);

    println!(
        "{} {}{} {}\n{}{}",
        now,
        json_type.to_string(),
        status_str,
        msg_string(msg_info),
        header_string(headers, context),
        color_treat(String::from(json), color),
    );
}

async fn get_random_packet_type(direction: PacketType, context: &SnoopContext) -> PacketType {
    match direction {
        PacketType::Request | PacketType::RequestDropped(_) => {
            if context.inner.drop_request_rate == 0.0 {
                PacketType::Request
            } else {
                let mut rng = context.inner.rng.lock().await;
                if rng.gen::<f32>() <= context.inner.drop_request_rate {
                    PacketType::RequestDropped(12.0)
                } else {
                    PacketType::Request
                }
            }
        }
        PacketType::Response | PacketType::ResponseDropped(_) => {
            if context.inner.drop_response_rate == 0.0 {
                PacketType::Response
            } else {
                let mut rng = context.inner.rng.lock().await;
                if rng.gen::<f32>() <= context.inner.drop_response_rate {
                    PacketType::ResponseDropped(12.0)
                } else {
                    PacketType::Response
                }
            }
        }
    }
}

fn copy_headers(headers: &HeaderMap<HeaderValue>) -> Vec<(HeaderName, HeaderValue)> {
    headers
        .iter()
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect()
}

fn suppress_log(
    message_type: PacketType,
    request_json: &String,
    request_path: &String,
    request_type: PacketType,
    response_type: PacketType,
    context: &SnoopContext,
) -> Option<(i32, String)> {
    if matches!(request_type, PacketType::RequestDropped(_))
        || matches!(response_type, PacketType::ResponseDropped(_))
    {
        // if either request or response is dropped, don't suppress
        return None;
    }
    if let Some((method, lines, suppress_type)) = serde_json::from_str::<RpcRequest>(request_json)
        .ok()
        .and_then(|rpc_request| {
            context.inner.suppress_method.as_ref().and_then(|method| {
                method
                    .get(&rpc_request.method)
                    .map(|(l, s)| (rpc_request.method.clone(), l, s))
            })
        })
    {
        if message_type.suppress(*suppress_type) {
            return Some((*lines, format!("[method {}]", method)));
        }
    }
    if let Some((lines, suppress_type)) = context
        .inner
        .suppress_path
        .as_ref()
        .and_then(|path| path.get(request_path))
    {
        if message_type.suppress(*suppress_type) {
            return Some((*lines, request_path.clone()));
        }
    }
    None
}

async fn handle_request(
    context: SnoopContext,
    _address: SocketAddr,
    source_request: Request<Body>,
) -> Result<Response<Body>, &'static str> {
    let request_path = source_request.uri().path().to_string();
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

    let request_type = get_random_packet_type(PacketType::Request, &context).await;
    let response_type = get_random_packet_type(PacketType::Response, &context).await;
    match suppress_log(
        PacketType::Request,
        &request_json,
        &request_path,
        request_type,
        response_type,
        &context,
    ) {
        Some((limit, _)) if limit < 0 => {}
        Some((limit, msg)) => print_json(
            &utils::trim_json(&request_json, limit),
            &request_headers,
            request_type,
            &msg,
            None,
            &context,
        ),
        None => print_json(
            &request_json,
            &request_headers,
            request_type,
            &request_path,
            None,
            &context,
        ),
    }

    if let PacketType::RequestDropped(delay) = request_type {
        let ms = (delay * 1000.0) as u64;
        sleep(Duration::from_millis(ms)).await;
        return Err("Request Dropped");
    }

    let (source_response, response_json) =
        if context.inner.override_rpc.is_some() && is_rpc_modules_request(&request_json) {
            get_rpc_modules_override(context.inner.override_rpc.as_ref().unwrap())
        } else {
            match get_response(dest_request, &context).await {
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
            }
        };
    let response_headers = copy_headers(source_response.headers());

    match suppress_log(
        PacketType::Response,
        &request_json,
        &request_path,
        request_type,
        response_type,
        &context,
    ) {
        Some((limit, _)) if limit < 0 => {}
        Some((limit, _msg)) => print_json(
            &utils::trim_json(&response_json, limit),
            &response_headers,
            response_type,
            "",
            Some(source_response.status()),
            &context,
        ),
        None => print_json(
            &response_json,
            &response_headers,
            response_type,
            "",
            Some(source_response.status()),
            &context,
        ),
    }

    if let PacketType::ResponseDropped(delay) = response_type {
        let ms = (delay * 1000.0) as u64;
        sleep(Duration::from_millis(ms)).await;
        return Err("Response Dropped");
    }

    Ok(source_response)
}

const SUPPRESS_HELP: &'static str = "
LINES=n specifies the degree of suppression:
    n < 0 Ignore message completely and log nothing [default]
    n = 0 Log that message occurred, but don't print any JSON
    n > 0 Log at most n lines of JSON
TYPE is one of:
    REQUEST:  Suppress request log
    RESPONSE: Suppress response log
    ALL:      Suppress both logs [default]";

#[tokio::main]
async fn main() {
    let matches = App::new("JSON-RPC Snooping Tool")
        .version("0.2")
        .author("Mark Mackey <ethereumdreamer@gmail.com>")
        .about("Proxies an http JSON-RPC endpoint and dumps requests and responses to screen")
        .arg(
            Arg::with_name("bind-address")
                .short('b')
                .long("bind-address")
                .help("Address to bind to and listen for incoming requests")
                .required(false)
                .default_value("127.0.0.1")
                .takes_value(true),
        )
        .arg(
            Arg::with_name("port")
                .short('p')
                .long("port")
                .help("Port to listen for incoming requests")
                .required(false)
                .default_value("3000")
                .takes_value(true),
        )
        .arg(
            Arg::with_name("log-headers")
                .short('l')
                .long("log-headers")
                .help("Print the headers in addition to request/response")
                .required(false)
                .takes_value(false),
        )
        .arg(
            Arg::with_name("no-color")
                .long("no-color")
                .short('n')
                .required(false)
                .takes_value(false)
                .help("Do not use terminal colors in output"),
        )
        .arg(
            Arg::with_name("suppress-method")
                .short('s')
                .long("suppress-method")
                .value_name("METHOD[:LINES][:TYPE]")
                .help("Suppress output of JSON RPC calls of this METHOD (can specify more than once)")
                .long_help(format!("Suppress output of JSON RPC calls of this METHOD (can specify more than once){}", SUPPRESS_HELP).as_str())
                .multiple(true)
                .number_of_values(1)
                .value_parser(utils::parse_suppress)
                .takes_value(true),
        )
        .arg(
            Arg::with_name("suppress-path")
                .short('S')
                .long("suppress-path")
                .value_name("PATH[:LINES][:TYPE]")
                .help("Suppress output of requests to the endpoint with this PATH (can specify more than once)")
                .long_help(format!("Suppress output of requests to the endpoint with this PATH (can specify more than once){}", SUPPRESS_HELP).as_str())
                .multiple(true)
                .number_of_values(1)
                .value_parser(utils::parse_suppress)
                .takes_value(true),
        )
        .arg(
            Arg::with_name("drop-request-rate")
                .long("drop-request-rate")
                .help("odds of randomly dropping a request for chaos testing [0..100]")
                .value_parser(clap::value_parser!(u32).range(0..101))
                .default_value("0")
                .takes_value(true)
        )
        .arg(
            Arg::with_name("drop-response-rate")
                .long("drop-response-rate")
                .help("odds of randomly dropping a response for chaos testing [0..100]")
                .value_parser(clap::value_parser!(u32).range(0..101))
                .default_value("0")
                .takes_value(true)
        )
        .arg(
            Arg::with_name("fix-geth-attach")
                .short('f')
                .long("fix-geth-attach")
                .help("Override the results of the `rpc_modules` method. This is useful for attaching a geth console to RPC endpoints that don't support the `rpc_modules` method (e.g. infura/nethermind by default)")
                .takes_value(false),
        )
        .arg(
            Arg::with_name("rpc-modules-override")
                .short('r')
                .long("rpc-modules-override")
                .requires("fix-geth-attach")
                .help("Specify a list of rpc modules to return from the `rpc_modules` method. Default [eth,net,web3]")
                .multiple(true)
                .number_of_values(1)
                .takes_value(true)
        )
        .arg(
            Arg::with_name("RPC_ENDPOINT")
                .help("JSON-RPC endpoint to forward incoming requests")
                .value_parser(utils::parse_uri)
                .required(true)
                .index(1),
        )
        .get_matches();

    let rng = match rand::rngs::StdRng::from_rng(rand::rngs::OsRng::default()) {
        Ok(rng) => rng,
        Err(e) => {
            eprintln!("Unable to initialize random number generator: {:?}", e);
            return;
        }
    };

    let context = SnoopContext {
        inner: Arc::new(Inner {
            dest_uri: matches.get_one::<Uri>("RPC_ENDPOINT").unwrap().clone(),
            rng: Mutex::new(rng),
            suppress_method: matches
                .get_many("suppress-method")
                .map(|iter| iter.cloned().collect()),
            suppress_path: matches
                .get_many("suppress-path")
                .map(|iter| iter.cloned().collect()),
            drop_request_rate: *matches.get_one::<u32>("drop-request-rate").unwrap() as f32 / 100.0,
            drop_response_rate: *matches.get_one::<u32>("drop-response-rate").unwrap() as f32
                / 100.0,
            override_rpc: matches
                .values_of("rpc-modules-override")
                .map(|values| values.into_iter().map(|s| s.to_string()).collect())
                .or(if matches.is_present("fix-geth-attach") {
                    Some(
                        vec!["eth", "net", "web3"]
                            .into_iter()
                            .map(Into::into)
                            .collect(),
                    )
                } else {
                    None
                }),
            colors: Colors::new(matches.is_present("no-color")),
            log_headers: matches.is_present("log-headers"),
        }),
    };

    // A `MakeService` that produces a `Service` to handle each connection.
    let make_service = make_service_fn(move |conn: &AddrStream| {
        let context = context.clone();
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
