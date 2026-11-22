//! HTTP client port (for `APIRequestContext`): one request, no redirects.

use std::io;

use futures::future::BoxFuture;

use crate::domain::http::{HttpRequest, HttpResponse};

/// Sends HTTP requests (a small HTTP/1.1 client in production).
pub trait HttpClient: Send + Sync {
    fn send(&self, request: HttpRequest) -> BoxFuture<'static, io::Result<HttpResponse>>;
}
