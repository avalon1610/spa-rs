//! This crate make a series of enhancements for [Axum](axum)
//!
use axum::{http::StatusCode, response::IntoResponse, response::Response};
use std::fmt::{Debug, Display};

pub mod filter;

/// The error type contains a [status code](StatusCode) and a string message.
///
/// It implements [IntoResponse], so can be used in [axum] handler.
///
/// # Example
/// 
/// ```
/// # use axum::response::IntoResponse;
/// # use axum_help::HttpError;
/// #
/// fn handler() -> Result<impl IntoResponse, HttpError> {
///     Ok(())
/// }
/// ```
/// Often it can to more convenient to use [HttpResult]
///
/// # Example
/// ```
/// # use axum::response::IntoResponse;
/// # use axum_help::HttpResult;
/// #
/// fn handler() -> HttpResult<impl IntoResponse> {
///     Ok(())
/// }
/// ```
#[derive(PartialEq, Debug)]
pub struct HttpError {
    pub message: String,
    pub status_code: StatusCode,
}

impl IntoResponse for HttpError {
    fn into_response(self) -> Response {
        let mut response = self.message.into_response();
        *response.status_mut() = self.status_code;
        response
    }
}

impl<E> From<E> for HttpError
where
    E: Debug + Display + Sync + Send + 'static,
{
    fn from(e: E) -> Self {
        Self {
            message: format!("{:?}", e),
            status_code: StatusCode::INTERNAL_SERVER_ERROR,
        }
    }
}

/// Construct an ad-hoc error from a string or existing error value.
///
/// This evaluates to an [HttpError]. It can take either just a string, or
/// a format string with arguments. It also can take any custom type
/// which implements Debug and Display.
///
/// If status code is not specified, [INTERNAL_SERVER_ERROR](StatusCode::INTERNAL_SERVER_ERROR)
/// will be used.
#[macro_export]
macro_rules! http_err {
    ($status: path, $fmt: literal, $($args: tt)+) => {
        HttpError {
            message: format!($fmt, $($args)+),
            status_code: $status
        }
    };
    ($status: path, $msg: literal) => {
        HttpError {
            message: $msg.to_string(),
            status_code: $status
        }
    };
    ($fmt: literal, $($args: tt)+) => {
        http_err!(StatusCode::INTERNAL_SERVER_ERROR, $fmt, $($args)+)
    };
    ($msg: literal) => {
        http_err!(StatusCode::INTERNAL_SERVER_ERROR, $msg)
    };
}

/// Return early with an [`HttpError`]
///
/// This macro is equivalent to `return Err(`[`http_err!($args...)`][http_err!]`)`.
///
/// The surrounding function's or closure's return value is required to be
/// Result<_, [`HttpError`]>
///
/// If status code is not specified, [INTERNAL_SERVER_ERROR](StatusCode::INTERNAL_SERVER_ERROR)
/// will be used.
///
/// # Example
///
/// ```
/// # use http::StatusCode;
/// # use axum_help::{http_bail, HttpError, http_err, HttpResult};
/// # use axum::response::IntoResponse;
/// #
/// fn get() -> HttpResult<()> {
///     http_bail!(StatusCode::BAD_REQUEST, "Bad Request: {}", "some reason");
/// }
/// ```
#[macro_export]
macro_rules! http_bail {
    ($($args: tt)+) => {
        return Err(http_err!($($args)+));
    };
}

/// Easily convert [std::result::Result] to [HttpResult]
///
/// # Example
/// ```
/// # use std::io::{Error, ErrorKind};
/// # use axum_help::{HttpResult, HttpContext};
/// # use http::StatusCode;
/// #
/// fn handler() -> HttpResult<()> {
/// #   let result = Err(Error::new(ErrorKind::InvalidInput, "bad input"));
///     result.http_context(StatusCode::BAD_REQUEST, "bad request")?;   
/// #   let result = Err(Error::new(ErrorKind::InvalidInput, "bad input"));
///     result.http_error("bad request")?;
/// 
///     Ok(())
/// }
/// ```
pub trait HttpContext<T> {
    fn http_context<C>(self, status_code: StatusCode, extra_msg: C) -> Result<T, HttpError>
    where
        C: Display + Send + Sync + 'static;

    fn http_error<C>(self, extra_msg: C) -> Result<T, HttpError>
    where
        C: Display + Send + Sync + 'static;
}

impl<T, E> HttpContext<T> for Result<T, E>
where
    E: Debug + Sync + Send + 'static,
{
    fn http_context<C>(self, status_code: StatusCode, extra_msg: C) -> Result<T, HttpError>
    where
        C: Display + Send + Sync + 'static,
    {
        self.map_err(|e| HttpError {
            message: format!("{}: {:?}", extra_msg, e),
            status_code,
        })
    }

    fn http_error<C>(self, extra_msg: C) -> Result<T, HttpError>
    where
        C: Display + Send + Sync + 'static,
    {
        self.map_err(|e| HttpError {
            message: format!("{}: {:?}", extra_msg, e),
            status_code: StatusCode::INTERNAL_SERVER_ERROR,
        })
    }
}

/// convenient return type when writing [axum] handler.
/// 
pub type HttpResult<T> = Result<T, HttpError>;

#[cfg(test)]
mod test {
    use super::HttpError;
    use axum::http::StatusCode;

    #[test]
    fn test_macros() -> Result<(), HttpError> {
        let error = HttpError {
            message: "aaa".to_string(),
            status_code: StatusCode::INTERNAL_SERVER_ERROR,
        };
        assert_eq!(error, http_err!(StatusCode::INTERNAL_SERVER_ERROR, "aaa"));
        assert_eq!(
            error,
            http_err!(StatusCode::INTERNAL_SERVER_ERROR, "{}aa", "a")
        );
        assert_eq!(error, http_err!("aaa"));
        assert_eq!(error, http_err!("{}aa", "a"));
        Ok(())
    }
}
