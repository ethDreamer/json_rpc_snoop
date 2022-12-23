use hyper::{
    http::{uri::InvalidUri, Error as HyperHttpError},
    Error as HyperError, Uri,
};
use serde::{Deserialize, Serialize};
use std::str::{FromStr, Utf8Error};

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
    pub id: u64,
    pub jsonrpc: String,
    pub method: String,
    pub params: Option<Vec<serde_json::Value>>,
}
#[derive(Debug, Deserialize, Serialize)]
pub struct RpcError {
    pub code: i32,
    pub message: String,
}
#[derive(Debug, Deserialize, Serialize)]
pub struct RpcErrorResponse {
    pub id: u64,
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

#[derive(Copy, Clone, PartialEq)]
pub enum PacketType {
    Request,
    Response,
    RequestDropped(f32),
    ResponseDropped(f32),
}

#[derive(Copy, Clone, Debug, Eq, Hash, PartialEq)]
pub enum SuppressType {
    Request,
    Response,
    All,
}

impl ToString for PacketType {
    fn to_string(&self) -> String {
        match self {
            PacketType::Request => "REQUEST".to_string(),
            PacketType::Response => "RESPONSE".to_string(),
            PacketType::RequestDropped(_wait) => {
                format!("DROPPED REQUEST")
            }
            PacketType::ResponseDropped(_wait) => {
                format!("DROPPED RESPONSE")
            }
        }
    }
}

impl FromStr for SuppressType {
    type Err = String;

    fn from_str(s: &str) -> Result<SuppressType, String> {
        match s.to_uppercase().as_str() {
            "REQUEST" => Ok(SuppressType::Request),
            "RESPONSE" => Ok(SuppressType::Response),
            "ALL" | "" => Ok(SuppressType::All),
            _ => Err(format!("Unable to parse '{}' as [REQUEST|RESPONSE|ALL]", s)),
        }
    }
}

impl PacketType {
    pub fn suppress(&self, st: SuppressType) -> bool {
        match st {
            SuppressType::Request => matches!(self, PacketType::Request),
            SuppressType::Response => matches!(self, PacketType::Response),
            SuppressType::All => true,
        }
    }
}

// remove trailing slashes from a str
pub fn remove_trailing_slashes(s: &str) -> &str {
    match s.char_indices().next_back() {
        Some((i, chr)) if chr == '/' => remove_trailing_slashes(&s[..i]),
        _ => s,
    }
}

pub fn trim_json(json: &str, limit: i32) -> String {
    let limit = limit as usize;
    let mut result = String::new();
    if limit > 0 {
        let lines = json.split("\n").collect::<Vec<_>>();
        let len = lines.len();
        if len <= limit {
            result = json.to_string();
        } else {
            let mut dots = false;
            for (i, line) in lines.into_iter().enumerate() {
                if i < limit / 2 {
                    result.push_str(line);
                    result.push_str("\n");
                } else if len - i > limit / 2 {
                    dots = true;
                } else {
                    if dots {
                        result.push_str("...\n");
                        dots = false;
                    }
                    result.push_str(line);
                    result.push_str("\n");
                }
            }
        }
    }
    result
}

pub fn parse_uri(s: &str) -> Result<Uri, InvalidUri> {
    remove_trailing_slashes(s).parse::<Uri>()
}

pub fn parse_suppress(arg: &str) -> Result<(String, (i32, SuppressType)), String> {
    let mut suppress = arg.to_string();
    let mut lines = -1;
    let mut suppress_type = SuppressType::All;
    for (i, s) in arg.split(":").enumerate() {
        match i {
            0 => suppress = s.to_string(),
            1 if !s.is_empty() => {
                lines = s
                    .parse()
                    .map_err(|e| format!("Unable to parse '{}' as LINES: {}", s, e))?
            }
            1 => {}
            2 => suppress_type = SuppressType::from_str(s)?,
            i if i > 2 => {
                return Err(format!(
                    "Unable to parse argument '{}' as 'PATH[:LINES][:TYPE]': too many colons",
                    arg
                ))
            }
            _ => unreachable!(),
        }
    }

    Ok((suppress, (lines, suppress_type)))
}
