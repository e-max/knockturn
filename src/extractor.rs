use crate::app::AppState;
use crate::db::GetMerchant;
use crate::errors::*;
use crate::jsonrpc;
use crate::models::Merchant;
use actix_identity::Identity;
use actix_session::Session as ActixSession;
use actix_web::dev;
use actix_web::{FromRequest, HttpRequest};
use actix_web_httpauth::extractors::basic;
use bytes::BytesMut;
use derive_deref::Deref;
use futures::future::{err, ok};
use futures::future::{Future, FutureExt, TryFutureExt};
use futures::stream::{Stream, StreamExt};
use serde::de::DeserializeOwned;
use std::default::Default;
use std::pin::Pin;

#[derive(Debug, Deref, Clone)]
pub struct BasicAuth<T>(pub T);

pub struct BasicAuthConfig(pub basic::Config);
impl Default for BasicAuthConfig {
    fn default() -> Self {
        BasicAuthConfig(basic::Config::default().realm("knocktrun"))
    }
}

impl FromRequest for BasicAuth<Merchant> {
    type Config = BasicAuthConfig;
    type Error = Error;
    type Future = Pin<Box<dyn Future<Output = Result<Self, Self::Error>> + 'static>>;

    fn from_request(req: &HttpRequest, _: &mut dev::Payload) -> Self::Future {
        let req = req.clone();

        async {
            let bauth = basic::BasicAuth::extract(&req)
                .await
                .map_err::<Error, _>(|e| Error::NotAuthorized)?;
            let data = req.app_data::<AppState>().unwrap();
            let username = bauth.user_id().to_string();
            let password = bauth.password().map(|p| p.to_string()).unwrap_or(s!(""));
            let merchant = data
                .db
                .send(GetMerchant { id: username })
                .await?
                .map_err(|e| Error::NotAuthorized)?;
            if merchant.token != password {
                Err(Error::NotAuthorized)
            } else {
                Ok(BasicAuth(merchant))
            }
        }
        .boxed_local()
    }
}

/// Session extractor
#[derive(Debug, Deref, Clone)]
pub struct Session<T>(pub T);

impl<T> Session<T> {
    pub fn into_inner(self) -> T {
        self.0
    }
}

pub struct SessionConfig(String);

impl Default for SessionConfig {
    fn default() -> Self {
        SessionConfig("merchant".to_owned())
    }
}

impl FromRequest for Session<Merchant> {
    type Config = SessionConfig;
    type Error = Error;
    type Future = Pin<Box<dyn Future<Output = Result<Self, Self::Error>>>>;

    fn from_request(req: &HttpRequest, _: &mut dev::Payload) -> Self::Future {
        let req = req.clone();
        let mut tmp;
        let cfg = if let Some(cfg) = req.app_data::<SessionConfig>() {
            cfg
        } else {
            tmp = SessionConfig::default();
            &tmp
        };
        async {
            let session = ActixSession::extract(&req)
                .await
                .map_err(|e| Error::NotAuthorizedInUI)?;
            let merchant_id = match session.get::<String>(&cfg.0) {
                Ok(Some(v)) => v,
                _ => return Err(Error::NotAuthorizedInUI),
            };
            let data = req.app_data::<AppState>().unwrap();
            let merchant = data
                .db
                .send(GetMerchant { id: merchant_id })
                .await?
                .map_err(|e| Error::NotAuthorizedInUI)?;
            Ok(Session(merchant))
        }
        .boxed_local()
    }
}

/// User extractor
#[derive(Debug, Deref, Clone)]
pub struct User<T>(pub T);

impl<T> User<T> {
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

impl FromRequest for User<Merchant> {
    type Config = IdentityConfig;
    type Error = Error;
    type Future = Pin<Box<dyn Future<Output = Result<Self, Self::Error>>>>;

    fn from_request(req: &HttpRequest, _: &mut dev::Payload) -> Self::Future {
        let req = req.clone();
        async {
            let id = Identity::extract(&req).await?;
            let merchant_id = id.identity().ok_or(Error::NotAuthorizedInUI)?;
            let data = req.app_data::<AppState>().unwrap();
            let merchant = data
                .db
                .send(GetMerchant { id: merchant_id })
                .await?
                .map_err(|e| Error::NotAuthorizedInUI)?;
            Ok(User(merchant))
        }
        .boxed_local()
    }
}

/// Json extractor
#[derive(Debug, Deref, Clone)]
pub struct SimpleJson<T>(pub T);

impl<T> SimpleJson<T> {
    pub fn into_inner(self) -> T {
        self.0
    }
}

pub struct SimpleJsonConfig;

impl Default for SimpleJsonConfig {
    fn default() -> Self {
        SimpleJsonConfig {}
    }
}
const MAX_SIZE: usize = 262_144 * 1024; // max payload size is 256m

impl<T> FromRequest for SimpleJson<T>
where
    T: DeserializeOwned + 'static,
{
    type Config = SimpleJsonConfig;
    type Error = Error;
    type Future = Pin<Box<dyn Future<Output = Result<Self, Self::Error>>>>;

    fn from_request(_: &HttpRequest, payload: &mut dev::Payload) -> Self::Future {
        let p = payload.take();
        use futures::stream::TryStreamExt;

        async {
            let body = BytesMut::new();
            while let Some(chunk) = p
                .try_next()
                .await
                .map_err(|e| Error::Internal(format!("Payload error: {:?}", e)))?
            {
                if (body.len() + chunk.len()) > MAX_SIZE {
                    return Err(Error::Internal("overflow".to_owned()));
                } else {
                    body.extend_from_slice(&chunk);
                }
            }
            let obj = serde_json::from_slice::<T>(&body)?;
            Ok(SimpleJson(obj))
        }
        .boxed_local()
    }
}

#[derive(Debug, Default)]
pub struct JsonRPCConfig {}

impl FromRequest for jsonrpc::Request {
    type Config = JsonRPCConfig;
    type Error = Error;
    type Future = Pin<Box<dyn Future<Output = Result<Self, Self::Error>>>>;

    fn from_request(_: &HttpRequest, payload: &mut dev::Payload) -> Self::Future {
        let p = payload.take();
        use futures::stream::TryStreamExt;

        async {
            let body = BytesMut::new();
            while let Some(chunk) = p
                .try_next()
                .await
                .map_err(|e| Error::Internal(format!("Payload error: {:?}", e)))?
            {
                if (body.len() + chunk.len()) > MAX_SIZE {
                    return Err(Error::Internal("overflow".to_owned()));
                } else {
                    body.extend_from_slice(&chunk);
                }
            }
            let req = serde_json::from_slice::<jsonrpc::Request>(&body)?;
            Ok(req)
        }
        .boxed_local()
    }
}
