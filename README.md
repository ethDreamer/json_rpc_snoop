# json_rpc_snoop

### How to build
Ensure you have cargo installed and in your PATH (the easiest way is to visit [https://rustup.rs/](https://rustup.rs/))
```
make
```

This will create the binary in `target/release/json_rpc_snoop`.

```
USAGE:
    json_rpc_snoop [OPTIONS] <RPC_ENDPOINT>

ARGS:
    <RPC_ENDPOINT>    JSON-RPC endpoint to forward incoming requests

OPTIONS:
    -b, --bind-address <bind-address>
            Address to bind to and listen for incoming requests [default: 127.0.0.1]

        --drop-request-rate <drop-request-rate>
            odds of randomly dropping a request for chaos testing [0..100] [default: 0]

        --drop-response-rate <drop-response-rate>
            odds of randomly dropping a response for chaos testing [0..100] [default: 0]

    -h, --help
            Print help information

    -l, --log-headers
            Print the headers in addition to request/response

    -n, --no-color
            Do not use terminal colors in output

    -p, --port <port>
            Port to listen for incoming requests [default: 3000]

    -s, --suppress-method <METHOD[:LINES][:TYPE]>
            Suppress output of JSON RPC calls of this METHOD (can specify more than once)

    -S, --suppress-path <PATH[:LINES][:TYPE]>
            Suppress output of requests to the endpoint with this PATH (can specify more than once)

    -V, --version
            Print version information
```

## Example Usage
If you have a JSON-RPC endpoint at `http://localhost:8545` and you want to run
the proxy on port `8560` and suppress successful `eth_getBlockByHash` requests
you would run:

```
./target/release/json_rpc_snoop -s eth_getBlockByHash -p 8560 http://localhost:8545
```

The two suppress options can be specified more than once and can accept a more
complicated syntax (run `json_rpc_snoop --help` to see FULL help output). For
example, to limit only the JSON **request** output to a maximum of 10 lines for
requests to the path `/eth/v1/builder/validators` you would pass:
```
--suppress-path /eth/v1/builder/validators:10:REQUEST
```

## Example Output

![example output png](https://i.imgur.com/NLzu4qo.png)

