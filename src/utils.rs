use hyper::http::Error as HyperHttpError;
use hyper::Error as HyperError;
use serde::{Deserialize, Serialize};
use std::str::Utf8Error;

#[derive(Debug)]
pub enum SnoopError {
    HyperError(HyperError),
    HyperHttpError(HyperHttpError),
    StringConversion(Utf8Error),
}

impl From<HyperHttpError> for SnoopError {
    fn from(e: HyperHttpError) -> Self {
        SnoopError::HyperHttpError(e)
    }
}

impl From<HyperError> for SnoopError {
    fn from(e: HyperError) -> Self {
        SnoopError::HyperError(e)
    }
}

impl From<Utf8Error> for SnoopError {
    fn from(e: Utf8Error) -> Self {
        SnoopError::StringConversion(e)
    }
}

#[derive(Debug, Deserialize, Serialize)]
pub struct RpcRequest {
    pub id: u8,
    pub jsonrpc: String,
    pub method: String,
    pub params: Vec<serde_json::Value>,
}
#[derive(Debug, Deserialize, Serialize)]
pub struct RpcError {
    pub code: i32,
    pub message: String,
}
#[derive(Debug, Deserialize, Serialize)]
pub struct RpcErrorResponse {
    pub id: u8,
    pub jsonrpc: String,
    pub error: RpcError,
}

impl From<(&str, SnoopError)> for RpcErrorResponse {
    fn from(pair: (&str, SnoopError)) -> Self {
        let (prefix, snoop_error) = pair;
        Self {
            id: 1,
            jsonrpc: "2.0".to_string(),
            error: RpcError {
                code: -32603,
                message: match snoop_error {
                    SnoopError::HyperError(e) => {
                        format!("{}: encountered hyper error: {:?}", prefix, e)
                    }
                    SnoopError::HyperHttpError(e) => {
                        format!("{}: encountered hyper::http error: {:?}", prefix, e)
                    }
                    SnoopError::StringConversion(e) => {
                        format!("{}: error converting to Utf-8: {:?}", prefix, e)
                    }
                },
            },
        }
    }
}
