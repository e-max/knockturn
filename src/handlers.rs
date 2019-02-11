use crate::app::AppState;
use crate::db::{CreateMerchant, CreateOrder, GetMerchantById};
use crate::errors::*;
use actix_web::{AsyncResponder, FutureResponse, HttpResponse, Json, Path, State};
use futures::future::Future;

pub fn create_merchant(
    (create_merchant, state): (Json<CreateMerchant>, State<AppState>),
) -> FutureResponse<HttpResponse> {
    state
        .db
        .send(create_merchant.into_inner())
        .from_err()
        .and_then(|db_response| {
            let merchant = db_response.map_err(|_| Error::DB)?;
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
            let merchant = db_response.map_err(|_| Error::DB)?;
            match merchant {
                Some(merchant) => Ok(HttpResponse::Ok().json(merchant)),
                None => Ok(HttpResponse::NotFound().json("")),
            }
        })
        .responder()
}

pub fn create_order(
    (create_order, state): (Json<CreateOrder>, State<AppState>),
) -> FutureResponse<HttpResponse> {
    state
        .db
        .send(create_order.into_inner())
        .from_err()
        .and_then(|db_response| {
            let order = db_response.map_err(|_| Error::DB)?;
            Ok(HttpResponse::Ok().json(order))
        })
        .responder()
}
