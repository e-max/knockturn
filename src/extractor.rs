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
        let res = async {
            match basic::BasicAuth::extract(&req).await {
                Ok(bauth) => {
                    let data = req.app_data::<AppState>().unwrap();
                    let username = bauth.user_id().to_string();
                    let password = bauth.password().map(|p| p.to_string()).unwrap_or(s!(""));
                    let resp: Result<_, Error> = data
                        .db
                        .send(GetMerchant { id: username })
                        .await
                        .map_err(|e| e.into())
                        .and_then(move |db_response| {
                            let merchant = match db_response {
                                Ok(m) => m,
                                Err(_) => return Err(Error::NotAuthorized),
                            };
                            if merchant.token != password {
                                Err(Error::NotAuthorized)
                            } else {
                                Ok(BasicAuth(merchant))
                            }
                        });
                    resp
                }
                Err(e) => Err(Error::NotAuthorized),
            }
        };

        Box::pin(res)
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

        let res = async {
            let r: Result<_, Error> = ActixSession::extract(&req).await.map_err(|e| e.into());

            let res2: Result<Session<Merchant>, Error> = match r {
                Ok(session) => {
                    let merchant_id = match session.get::<String>(&cfg.0) {
                        Ok(Some(v)) => v,
                        _ => return Err(Error::NotAuthorizedInUI),
                    };
                    let data = req.app_data::<AppState>().unwrap();
                    data.db
                        .send(GetMerchant { id: merchant_id })
                        .await
                        .map_err(|e| e.into())
                        .and_then(move |db_response| match db_response {
                            Ok(m) => Ok(Session(m)),
                            Err(_) => Err(Error::NotAuthorizedInUI),
                        })
                }
                Err(e) => Err(e),
            };
            res2
        };

        Box::pin(res)
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
        let res = async {
            let r: Result<Identity, Error> = Identity::extract(&req).await.map_err(|e| e.into());
            let id: Identity = match r {
                Ok(v) => v,
                _ => return Err::<User<Merchant>, Error>(Error::NotAuthorizedInUI),
            };
            let merchant_id = match id.identity() {
                Some(v) => v,
                _ => return Err::<User<Merchant>, Error>(Error::NotAuthorizedInUI),
            };
            let data = req.app_data::<AppState>().unwrap();
            data.db
                .send(GetMerchant { id: merchant_id })
                .await
                .map_err(|e| e.into())
                .and_then(move |db_response| match db_response {
                    Ok(m) => Ok(User(m)),
                    Err(_) => Err(Error::NotAuthorizedInUI),
                })
        };
        Box::pin(res)

        /*
        let id = match Identity::extract(req) {
            Ok(v) => v,
            _ => return Box::new(err(Error::NotAuthorizedInUI)),
        };

        let merchant_id = match id.identity() {
            Some(v) => v,
            _ => return Box::new(err(Error::NotAuthorizedInUI)),
        };

        let data = req.app_data::<AppState>().unwrap();
        Box::new(
            data.db
                .send(GetMerchant { id: merchant_id })
                .from_err()
                .and_then(move |db_response| match db_response {
                    Ok(m) => ok(User(m)),
                    Err(_) => err(Error::NotAuthorizedInUI),
                }),
        )
        */
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

        let res = async {
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
        };
        res.boxed_local()
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

        let res = async {
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
        };
        res.boxed_local()

        //Box::new(
        //payload
        //.take()
        //.map_err(|e| Error::Internal(format!("Payload error: {:?}", e)))
        //.fold(BytesMut::new(), move |mut body, chunk| {
        //if (body.len() + chunk.len()) > MAX_SIZE {
        //Err(Error::Internal("overflow".to_owned()))
        //} else {
        //body.extend_from_slice(&chunk);
        //Ok(body)
        //}
        //})
        //.and_then(|body| {
        //let req = serde_json::from_slice::<jsonrpc::Request>(&body)?;
        //Ok(req)
        //}),
        //)
    }
}
