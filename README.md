# json_rpc_snoop

### How to build
Ensure you have cargo installed and in your PATH (the easiest way is to visit [https://rustup.rs/](https://rustup.rs/))
```
make
```

This will create the binary in `target/release/json_rpc_snoop`.

```
USAGE:
    json_rpc_snoop [FLAGS] [OPTIONS] <RPC_ENDPOINT>

FLAGS:
    -h, --help        Prints help information
    -n, --no-color    Do not use terminal colors in output
    -V, --version     Prints version information

OPTIONS:
    -b, --bind-address <bind-address>       Address to bind to and listen for incoming requests [default: 127.0.0.1]
    -p, --port <port>                       Port to listen for incoming requests [default: 3000]
    -S, --suppress-all <suppress-all>...    Suppress success or error JSON RPC method calls of this type in output (can
                                            specify more than one)
    -s, --suppress-ok <suppress-ok>...      Suppress successful JSON RPC method calls of this type in output (can
                                            specify more than one)

ARGS:
    <RPC_ENDPOINT>    JSON-RPC endpoint to forward incoming requests
```

## Example
If you have a JSON-RPC endpoint at `http://localhost:8545` and you want to run
the proxy on port 8560 and suppress successful `eth_getBlockByHash` requests
you would run:

```
./target/release/json_rpc_snoop -s eth_getBlockByHash -p 8560 http://localhost:8545
```

