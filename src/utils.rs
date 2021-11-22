use hyper::http::Error as HyperHttpError;
use hyper::Error as HyperError;
use serde_json::Error as SerdeError;
use std::str::Utf8Error;

#[derive(Debug)]
pub enum SnoopError {
    HyperError(HyperError),
    HyperHttpError(HyperHttpError),
    StringConversion(Utf8Error),
    SerdeError(SerdeError),
    Json(String),
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

impl From<SerdeError> for SnoopError {
    fn from(e: SerdeError) -> Self {
        SnoopError::SerdeError(e)
    }
}

impl From<String> for SnoopError {
    fn from(err_str: String) -> Self {
        SnoopError::Json(err_str)
    }
}
