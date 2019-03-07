use crate::app::AppState;
use actix_web::middleware::identity::RequestIdentity;
use actix_web::middleware::{Middleware, Started};
use actix_web::{
    http::header, http::Method, server, App, HttpRequest, HttpResponse, ResponseError,
};

pub struct SiteAuthMiddleware;

impl Middleware<AppState> for SiteAuthMiddleware {
    fn start(&self, req: &HttpRequest<AppState>) -> actix_web::Result<Started> {
        if req.identity().is_none() {
            Ok(Started::Response(
                HttpResponse::Found()
                    .header(header::LOCATION, "/login")
                    .finish(),
            ))
        } else {
            Ok(Started::Done)
        }
    }
}
