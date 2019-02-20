use crate::app::AppState;
use crate::db::{CreateMerchant, CreateOrder, GetMerchant, GetOrder};
use crate::errors::*;
use crate::models::{Money, OrderStatus};
use actix_web::{AsyncResponder, FutureResponse, HttpResponse, Json, Path, State};
use askama::Template;
use bcrypt;
use enum_primitive::FromPrimitive;
use futures::future::{result, Future};
use serde::Deserialize;

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
    pub amount: i64,
    pub currency: String,
    pub confirmations: i32,
    pub callback_url: String,
    pub email: Option<String>,
}

pub fn create_order(
    (merchant_id, order_req, state): (Path<String>, Json<CreateOrderRequest>, State<AppState>),
) -> FutureResponse<HttpResponse> {
    let create_order = CreateOrder {
        merchant_id: merchant_id.into_inner(),
        order_id: order_req.order_id.clone(),
        amount: order_req.amount,
        currency: order_req.currency.clone(),
        confirmations: order_req.confirmations,
        callback_url: order_req.callback_url.clone(),
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
            let amount = Money {
                amount: order.amount,
                currency: order.currency,
            };
            let html = OrderTemplate {
                order_id: order.order_id,
                merchant_id: order.merchant_id,
                amount,
                confirmations: order.confirmations,
                grins: Money {
                    amount: order.grin_amount,
                    currency: "grin".to_owned(),
                },
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
    grins: Money,
    confirmations: i32,
}
