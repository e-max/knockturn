use crate::app::AppState;
use crate::db::GetMerchant;
use crate::errors::*;
use crate::models::Merchant;
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
    type Result = Box<Future<Item = Self, Error = Error>>;

    fn from_request(req: &HttpRequest<AppState>, cfg: &Self::Config) -> Self::Result {
        let auth = basic::BasicAuth::from_request(&req, &cfg.0)?;
        let password = match auth.password() {
            Some(p) => p,
            None => return Box::new(err(Error::NotAuthorized)),
        };

        req.state()
            .db
            .send(GetMerchant { id: auth.login() })
            .from_err()
            .and_then(move |db_response| {
                let merchant = db_response?;
                match bcrypt::verify(password, &merchant.password) {
                    Err(_) => Err(Error::NotAuthorized),
                    Ok(res) => {
                        if res {
                            Ok(BasicAuth(merchant))
                        } else {
                            Err(Error::NotAuthorized)
                        }
                    }
                }
            })
    }
}
