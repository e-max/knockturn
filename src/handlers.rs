use crate::app::AppState;
use crate::db::{CreateMerchant, CreateOrder, GetMerchantById};
use crate::errors::*;
use actix_web::{AsyncResponder, FutureResponse, HttpResponse, Json, Path, State};
use futures::future::Future;
use serde::Deserialize;

pub fn create_merchant(
    (create_merchant, state): (Json<CreateMerchant>, State<AppState>),
) -> FutureResponse<HttpResponse> {
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
        .send(GetMerchantById {
            id: merchant_id.to_owned(),
        })
        .from_err()
        .and_then(|db_response| {
            let merchant = db_response?;
            match merchant {
                Some(merchant) => Ok(HttpResponse::Ok().json(merchant)),
                None => Ok(HttpResponse::NotFound().json("")),
            }
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
