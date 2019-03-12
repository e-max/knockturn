use crate::app::AppState;
use crate::db::GetMerchant;
//use crate::errors::*;
use crate::models::Merchant;
use actix_web::error::{Error, ErrorUnauthorized};
use actix_web::{FromRequest, HttpRequest, HttpResponse};
use actix_web_httpauth::extractors::basic;
use bcrypt;
use futures::future::{err, ok, result, Future};
use std::default::Default;
use std::ops::{Deref, DerefMut};

pub struct BasicAuth<T>(pub T);

impl<T> BasicAuth<T> {
    pub fn into_inner(self) -> T {
        self.0
    }
}

impl<T> Deref for BasicAuth<T> {
    type Target = T;

    fn deref(&self) -> &T {
        &self.0
    }
}

impl<T> DerefMut for BasicAuth<T> {
    fn deref_mut(&mut self) -> &mut T {
        &mut self.0
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
        let bauth = basic::BasicAuth::from_request(&req, &cfg.0)
            .map_err(|_| ErrorUnauthorized("auth error: request"))?;
        let username = bauth.username().to_owned();

        Ok(Box::new(
            req.state()
                .db
                .send(GetMerchant { id: username })
                .from_err()
                .and_then(move |db_response| {
                    let merchant = match db_response {
                        Ok(m) => m,
                        Err(e) => return err(ErrorUnauthorized(e)),
                    };
                    let password = bauth.password().unwrap_or("");
                    if merchant.token != password {
                        err(ErrorUnauthorized(format!(
                            "auth error exp {} got {}",
                            merchant.token, password
                        )))
                    } else {
                        ok(BasicAuth(merchant))
                    }
                }),
        ))
    }
}
