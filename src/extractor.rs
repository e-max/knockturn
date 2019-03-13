use crate::app::AppState;
use crate::db::GetMerchant;
use crate::errors::*;
use crate::models::Merchant;
use actix_web::middleware::identity::RequestIdentity;
use actix_web::{FromRequest, HttpRequest};
use actix_web_httpauth::extractors::basic;
use derive_deref::Deref;
use futures::future::{err, ok, Future};
use std::default::Default;

#[derive(Debug, Deref, Clone)]
pub struct BasicAuth<T>(pub T);

impl<T> BasicAuth<T> {
    pub fn into_inner(self) -> T {
        self.0
    }
}

pub struct BasicAuthConfig(pub basic::Config);
impl Default for BasicAuthConfig {
    fn default() -> Self {
        let mut config = basic::Config::default();
        config.realm("knocktrun");
        BasicAuthConfig(config)
    }
}

impl FromRequest<AppState> for BasicAuth<Merchant> {
    type Config = BasicAuthConfig;
    type Result = Result<Box<dyn Future<Item = Self, Error = Error>>, Error>;

    fn from_request(req: &HttpRequest<AppState>, cfg: &Self::Config) -> Self::Result {
        let bauth =
            basic::BasicAuth::from_request(&req, &cfg.0).map_err(|_| Error::NotAuthorized)?;
        let username = bauth.username().to_owned();

        Ok(Box::new(
            req.state()
                .db
                .send(GetMerchant { id: username })
                .from_err()
                .and_then(move |db_response| {
                    let merchant = match db_response {
                        Ok(m) => m,
                        Err(e) => return err(Error::NotAuthorized),
                    };
                    let password = bauth.password().unwrap_or("");
                    if merchant.token != password {
                        err(Error::NotAuthorized)
                    } else {
                        ok(BasicAuth(merchant))
                    }
                }),
        ))
    }
}

/// Identity extractor
#[derive(Debug, Deref, Clone)]
pub struct Identity<T>(pub T);

impl<T> Identity<T> {
    pub fn into_inner(self) -> T {
        self.0
    }
}

pub struct IdentityConfig;

impl Default for IdentityConfig {
    fn default() -> Self {
        IdentityConfig {}
    }
}

impl FromRequest<AppState> for Identity<Merchant> {
    type Config = IdentityConfig;
    type Result = Result<Box<dyn Future<Item = Self, Error = Error>>, Error>;

    fn from_request(req: &HttpRequest<AppState>, cfg: &Self::Config) -> Self::Result {
        let merchant_id = match req.identity() {
            Some(v) => v,
            None => return Err(Error::NotAuthorizedInUI),
        };

        Ok(Box::new(
            req.state()
                .db
                .send(GetMerchant { id: merchant_id })
                .from_err()
                .and_then(move |db_response| match db_response {
                    Ok(m) => ok(Identity(m)),
                    Err(e) => err(Error::NotAuthorizedInUI),
                }),
        ))
    }
}
