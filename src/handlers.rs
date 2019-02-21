use crate::app::AppState;
use crate::db::{CreateMerchant, CreateOrder, CreateTx, GetMerchant, GetOrder, GetOrders};
use crate::errors::*;
use crate::models::{Currency, Money, Order, OrderStatus};
use crate::wallet::Slate;
use actix_web::middleware::identity::RequestIdentity;
use actix_web::{
    AsyncResponder, Form, FromRequest, FutureResponse, HttpRequest, HttpResponse, Json, Path,
    Responder, State,
};
use askama::Template;
use bcrypt;
use enum_primitive::FromPrimitive;
use futures::future::{err, ok, result, Future};
use log::debug;
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
                        req.remember(merchant.id);
                        Ok(HttpResponse::Found().header("location", "/").finish())
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
        order_id: order_req.order_id.clone(),
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
                order_id: order.order_id,
                merchant_id: order.merchant_id,
                amount: order.amount,
                confirmations: order.confirmations,
                grin_amount: Money::new(order.grin_amount, Currency::GRIN),
                status: OrderStatus::from_i32(order.status).unwrap().to_string(),
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

pub fn pay_order(
    (order, slate, state): (Path<GetOrder>, Json<Slate>, State<AppState>),
) -> FutureResponse<HttpResponse, Error> {
    let slate = state.wallet.receive(&slate);

    slate
        .and_then(move |slate| {
            state
                .wallet
                .get_tx(&slate.id.to_hyphenated().to_string())
                .and_then(move |tx| {
                    let messages: Vec<String> = if let Some(pm) = tx.messages {
                        pm.messages
                            .into_iter()
                            .map(|pmd| pmd.message)
                            .filter_map(|x| x)
                            .collect()
                    } else {
                        vec![]
                    };

                    let msg = CreateTx {
                        slate_id: tx.tx_slate_id.unwrap(),
                        created_at: tx.creation_ts,
                        confirmed: tx.confirmed,
                        confirmed_at: tx.confirmation_ts,
                        fee: tx.fee.map(|f| f as i64),
                        messages: messages,
                        num_inputs: tx.num_inputs as i64,
                        num_outputs: tx.num_outputs as i64,
                        //FIXME
                        tx_type: format!("{:?}", tx.tx_type),
                        order_id: order.order_id.clone(),
                        merchant_id: order.merchant_id.clone(),
                    };
                    state.db.send(msg).from_err()
                })
                .and_then(|_| ok(slate))
        })
        .and_then(|slate| Ok(HttpResponse::Ok().json(slate)))
        .responder()
}
