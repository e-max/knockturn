use crate::app::AppState;
use crate::db::GetMerchant;
use crate::models::Merchant;
use actix_web::middleware::identity::RequestIdentity;
use actix_web::middleware::{Middleware, Started};
use actix_web::{http::header, HttpRequest, HttpResponse};
use futures::future::Future;

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

pub struct MerchantMiddleware;

pub struct MerchantBox(Merchant);

impl Middleware<AppState> for MerchantMiddleware {
    fn start(&self, req: &HttpRequest<AppState>) -> actix_web::Result<Started> {
        if let Some(id) = req.identity() {
            let res = req
                .state()
                .db
                .send(GetMerchant { id })
                .from_err()
                .and_then({
                    let req = req.clone();
                    move |db_response| {
                        let merchant = db_response?;
                        req.extensions_mut().insert(MerchantBox(merchant));
                        Ok(None)
                    }
                });
            return Ok(Started::Future(Box::new(res)));
        }
        Ok(Started::Done)
    }
}

pub trait WithMerchant {
    fn get_merchant(&self) -> Option<Merchant>;
}

impl WithMerchant for HttpRequest<AppState> {
    fn get_merchant(&self) -> Option<Merchant> {
        self.extensions().get().map(|mb: &MerchantBox| mb.0.clone())
    }
}
