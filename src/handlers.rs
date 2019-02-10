use crate::app::AppState;
use crate::db::{CreateMerchant, GetMerchantById};
use crate::errors::*;
use actix_web::{AsyncResponder, FutureResponse, HttpResponse, Json, Path, ResponseError, State};
use futures::future::Future;
use uuid::Uuid;

pub fn create_merchant(
    (create_merchant, state): (Json<CreateMerchant>, State<AppState>),
) -> FutureResponse<HttpResponse> {
    state
        .db
        .send(create_merchant.into_inner())
        .from_err()
        .and_then(|db_response| {
            let merchant = db_response.map_err(|_| ServiceError::InternalServerError)?;
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
            let merchant = db_response.map_err(|_| ServiceError::InternalServerError)?;
            match merchant {
                Some(merchant) => Ok(HttpResponse::Ok().json(merchant)),
                None => Ok(HttpResponse::NotFound().json("")),
            }
        })
        .responder()
}
