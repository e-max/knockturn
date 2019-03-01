use crate::app::AppState;
use crate::db::{
    Confirm2FA, CreateMerchant, CreateOrder, CreateTx, GetMerchant, GetOrder, GetOrders,
    UpdateOrderStatus,
};
use crate::errors::*;
use crate::fsm::{GetUnpaidOrder, PayOrder};
use crate::models::{Currency, Money, Order, OrderStatus};
use crate::totp::Totp;
use crate::wallet::Slate;
use actix_web::http::Method;
use actix_web::middleware::identity::RequestIdentity;
use actix_web::middleware::session::RequestSession;
use actix_web::{
    error, AsyncResponder, Form, FromRequest, FutureResponse, HttpMessage, HttpRequest,
    HttpResponse, Json, Path, Responder, State,
};
use askama::Template;
use bcrypt;
use bytes::BytesMut;
use data_encoding::BASE64;
use futures::future::{err, ok, result, Future};
use futures::stream::Stream;
use log::debug;
use rand::Rng;
use serde::Deserialize;
use std::iter::Iterator;

#[derive(Template)]
#[template(path = "index.html")]
struct IndexTemplate<'a> {
    merchant_id: &'a str,
    orders: Vec<Order>,
}

pub fn index(req: &HttpRequest<AppState>) -> FutureResponse<HttpResponse> {
    let merchant_id = match req.identity() {
        Some(v) => v,
        None => return ok(HttpResponse::Found().header("location", "/login").finish()).responder(),
    };
    req.state()
        .db
        .send(GetOrders {
            merchant_id: merchant_id.clone(),
            offset: 0,
            limit: 100,
        })
        .from_err()
        .and_then(move |db_response| {
            let orders = db_response?;
            let html = IndexTemplate {
                merchant_id: &merchant_id,
                orders,
            }
            .render()
            .map_err(|e| Error::from(e))?;
            Ok(HttpResponse::Ok().content_type("text/html").body(html))
        })
        .responder()
}

#[derive(Debug, Deserialize)]
pub struct LoginRequest {
    pub login: String,
    pub password: String,
}
pub fn login(
    (req, login_form): (HttpRequest<AppState>, Form<LoginRequest>),
) -> FutureResponse<HttpResponse> {
    req.state()
        .db
        .send(GetMerchant {
            id: login_form.login.clone(),
        })
        .from_err()
        .and_then(move |db_response| {
            let merchant = db_response?;
            match bcrypt::verify(&login_form.password, &merchant.password) {
                Ok(res) => {
                    if res {
                        req.session().set("merchant", merchant.id)?;
                        if merchant.confirmed_2fa {
                            Ok(HttpResponse::Found().header("location", "/2fa").finish())
                        } else {
                            Ok(HttpResponse::Found()
                                .header("location", "/set_2fa")
                                .finish())
                        }
                    } else {
                        Ok(HttpResponse::Found().header("location", "/login").finish())
                    }
                }
                Err(_) => Ok(HttpResponse::Found().header("location", "/login").finish()),
            }
        })
        .responder()
}

pub fn login_form(req: HttpRequest<AppState>) -> HttpResponse {
    HttpResponse::Ok()
        .content_type("text/html; charset=utf-8")
        .body(include_str!("../templates/login.html"))
}

pub fn logout(req: HttpRequest<AppState>) -> Result<HttpResponse, Error> {
    req.forget();
    req.session().clear();
    Ok(HttpResponse::Found().header("location", "/login").finish())
}

pub fn create_merchant(
    (mut create_merchant, state): (Json<CreateMerchant>, State<AppState>),
) -> FutureResponse<HttpResponse> {
    create_merchant.password = match bcrypt::hash(&create_merchant.password, bcrypt::DEFAULT_COST) {
        Ok(v) => v,
        Err(_) => return result(Ok(HttpResponse::InternalServerError().finish())).responder(),
    };
    state
        .db
        .send(create_merchant.into_inner())
        .from_err()
        .and_then(|db_response| {
            let merchant = db_response?;
            Ok(HttpResponse::Ok().json(merchant))
        })
        .responder()
}

pub fn get_merchant(
    (merchant_id, state): (Path<String>, State<AppState>),
) -> FutureResponse<HttpResponse> {
    state
        .db
        .send(GetMerchant {
            id: merchant_id.to_owned(),
        })
        .from_err()
        .and_then(|db_response| {
            let merchant = db_response?;
            Ok(HttpResponse::Ok().json(merchant))
        })
        .responder()
}

#[derive(Debug, Deserialize)]
pub struct CreateOrderRequest {
    pub order_id: String,
    pub amount: Money,
    pub confirmations: i32,
    pub email: Option<String>,
}

pub fn create_order(
    (merchant_id, order_req, state): (Path<String>, Json<CreateOrderRequest>, State<AppState>),
) -> FutureResponse<HttpResponse> {
    let create_order = CreateOrder {
        merchant_id: merchant_id.into_inner(),
        external_id: order_req.order_id.clone(),
        amount: order_req.amount,
        confirmations: order_req.confirmations,
        email: order_req.email.clone(),
    };
    state
        .db
        .send(create_order)
        .from_err()
        .and_then(|db_response| {
            let order = db_response?;

            Ok(HttpResponse::Ok().json(order))
        })
        .responder()
}

pub fn get_order(
    (get_order, state): (Path<GetOrder>, State<AppState>),
) -> FutureResponse<HttpResponse> {
    state
        .db
        .send(get_order.into_inner())
        .from_err()
        .and_then(|db_response| {
            let order = db_response?;
            let html = OrderTemplate {
                order_id: order.id.to_string(),
                merchant_id: order.merchant_id,
                amount: order.amount,
                confirmations: order.confirmations,
                grin_amount: Money::new(order.grin_amount, Currency::GRIN),
                status: order.status.to_string(),
            }
            .render()
            .map_err(|e| Error::from(e))?;
            Ok(HttpResponse::Ok().content_type("text/html").body(html))
        })
        .responder()
}

#[derive(Template)]
#[template(path = "order.html")]
struct OrderTemplate {
    order_id: String,
    merchant_id: String,
    status: String,
    amount: Money,
    grin_amount: Money,
    confirmations: i32,
}

pub fn get_tx(state: State<AppState>) -> Box<Future<Item = HttpResponse, Error = Error>> {
    state
        .wallet
        .get_tx("c3b4be4a-b72c-46f5-8fb0-e318ca19ba2b")
        .and_then(|body| Ok(HttpResponse::Ok().json(body)))
        .responder()
}

const MAX_SIZE: usize = 262_144; // max payload size is 256k

#[derive(Template)]
#[template(path = "totp.html")]
struct TotpTemplate<'a> {
    msg: &'a str,
    token: &'a str,
    image: &'a str,
}

#[derive(Debug, Deserialize)]
pub struct TotpRequest {
    pub code: String,
}

pub fn get_totp(req: HttpRequest<AppState>) -> FutureResponse<HttpResponse, Error> {
    let merchant_id = match req.session().get::<String>("merchant") {
        Ok(Some(v)) => v,
        _ => {
            return Box::new(ok(HttpResponse::Found()
                .header("location", "/login")
                .finish()));
        }
    };
    req.state()
        .db
        .send(GetMerchant {
            id: merchant_id.clone(),
        })
        .from_err()
        .and_then(move |db_response| {
            let merchant = db_response?;

            let token = merchant
                .token_2fa
                .ok_or(Error::General(s!("No 2fa token")))?;
            let totp = Totp::new(merchant.id.clone(), token.clone());

            let html = TotpTemplate {
                msg: "",
                token: &token,
                image: &BASE64.encode(&totp.get_png()?),
            }
            .render()
            .map_err(|e| Error::from(e))?;
            let resp = HttpResponse::Ok().content_type("text/html").body(html);
            Ok(resp)
        })
        .responder()
}

pub fn post_totp(
    (req, totp_form): (HttpRequest<AppState>, Form<TotpRequest>),
) -> FutureResponse<HttpResponse, Error> {
    let merchant_id = match req.session().get::<String>("merchant") {
        Ok(Some(v)) => v,
        _ => {
            return Box::new(ok(HttpResponse::Found()
                .header("location", "/login")
                .finish()));
        }
    };
    req.state()
        .db
        .send(GetMerchant {
            id: merchant_id.clone(),
        })
        .from_err()
        .and_then({
            let db = req.state().db.clone();
            let request_method = req.method().clone();
            move |db_response| {
                let merchant = db_response?;

                let mut msg = String::new();

                let token = merchant
                    .token_2fa
                    .ok_or(Error::General(s!("No 2fa token")))?;
                let totp = Totp::new(merchant.id.clone(), token.clone());

                if request_method == Method::POST {
                    if totp.check(&totp_form.code)? {
                        let resp = HttpResponse::Found().header("location", "/").finish();
                        return Ok((true, resp));
                    }
                    msg.push_str("Incorrect code, please try one more time");
                }

                let html = TotpTemplate {
                    msg: &msg,
                    token: &token,
                    //image: &BASE64.encode(&svg_xml.as_bytes()),
                    image: &BASE64.encode(&totp.get_png()?),
                }
                .render()
                .map_err(|e| Error::from(e))?;
                let resp = HttpResponse::Ok().content_type("text/html").body(html);
                Ok((false, resp))
            }
        })
        .and_then({
            let db = req.state().db.clone();
            move |(confirm, response)| {
                db.send(Confirm2FA { merchant_id })
                    .from_err()
                    .and_then(move |db_response| {
                        db_response?;
                        Ok(response)
                    })
            }
        })
        .responder()
}

pub fn get_qrcode(state: State<AppState>) -> Result<HttpResponse, Error> {
    Ok(HttpResponse::Ok().content_type("image/svg+xml;").body(""))
}

pub fn pay_order(
    (order, req, state): (Path<GetUnpaidOrder>, HttpRequest<AppState>, State<AppState>),
) -> FutureResponse<HttpResponse, Error> {
    req.payload()
        .map_err(|e| Error::Internal(format!("Payload error: {:?}", e)))
        //.from_err()
        .fold(BytesMut::new(), move |mut body, chunk| {
            if (body.len() + chunk.len()) > MAX_SIZE {
                Err(Error::Internal("overflow".to_owned()))
            } else {
                body.extend_from_slice(&chunk);
                Ok(body)
            }
        })
        .and_then(|body| {
            let slate = serde_json::from_slice::<Slate>(&body)?;
            Ok(slate)
        })
        .and_then(move |slate| {
            let slate_amount = slate.amount;
            state
                .fsm
                .send(order.into_inner())
                .from_err()
                .and_then(move |db_response| {
                    let unpaid_order = db_response?;
                    if unpaid_order.grin_amount != slate_amount as i64 {
                        return Err(Error::WrongAmount(
                            unpaid_order.grin_amount as u64,
                            slate_amount,
                        ));
                    }
                    Ok(unpaid_order)
                })
                .and_then({
                    let wallet = state.wallet.clone();
                    let fsm = state.fsm.clone();
                    move |unpaid_order| {
                        let slate = wallet.receive(&slate);
                        slate.and_then(move |slate| {
                            wallet
                                .get_tx(&slate.id.hyphenated().to_string())
                                .and_then(move |wallet_tx| {
                                    fsm.send(PayOrder {
                                        unpaid_order,
                                        wallet_tx,
                                    })
                                    .from_err()
                                    .and_then(|db_response| {
                                        db_response?;
                                        Ok(())
                                    })
                                })
                                .and_then(|_| ok(slate))
                        })
                    }
                })
                .and_then(|slate| Ok(HttpResponse::Ok().json(slate)))
        })
        .responder()
}

pub fn form_2fa(req: HttpRequest<AppState>) -> HttpResponse {
    HttpResponse::Ok()
        .content_type("text/html; charset=utf-8")
        .body(include_str!("../templates/2fa.html"))
}

pub fn post_2fa(
    (req, totp_form): (HttpRequest<AppState>, Form<TotpRequest>),
) -> FutureResponse<HttpResponse, Error> {
    let merchant_id = match req.session().get::<String>("merchant") {
        Ok(Some(v)) => v,
        _ => {
            return Box::new(ok(HttpResponse::Found()
                .header("location", "/login")
                .finish()));
        }
    };
    req.state()
        .db
        .send(GetMerchant {
            id: merchant_id.clone(),
        })
        .from_err()
        .and_then({
            let db = req.state().db.clone();
            let request_method = req.method().clone();
            move |db_response| {
                let merchant = db_response?;

                let token = merchant
                    .token_2fa
                    .ok_or(Error::General(s!("No 2fa token")))?;
                let totp = Totp::new(merchant.id.clone(), token.clone());

                if totp.check(&totp_form.code)? {
                    req.remember(merchant.id);
                    return Ok(HttpResponse::Found().header("location", "/").finish());
                } else {
                    Ok(HttpResponse::Found().header("location", "/2fa").finish())
                }
            }
        })
        .responder()
}
