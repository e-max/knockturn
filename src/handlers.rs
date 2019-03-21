use crate::app::AppState;
use crate::blocking;
use crate::db::{
    Confirm2FA, CreateMerchant, GetCurrentHeight, GetMerchant, GetTransaction, GetTransactions,
};
use crate::errors::*;
use crate::extractor::{BasicAuth, Identity, Session, SimpleJson};
use crate::filters;
use crate::fsm::{
    CreatePayment, CreatePayout, FinalizePayout, GetInitializedPayout, GetNewPayment, GetNewPayout,
    GetPayout, InitializePayout, MakePayment, PayoutFees,
};
use crate::fsm::{KNOCKTURN_SHARE, MINIMAL_WITHDRAW, TRANSFER_FEE};
use crate::middleware::WithMerchant;
use crate::models::{Currency, Merchant, Money, Transaction, TransactionStatus, TransactionType};
use crate::qrcode;
use crate::totp::Totp;
use crate::wallet::Slate;
use actix_web::http::Method;
use actix_web::middleware::identity::RequestIdentity;
use actix_web::middleware::session::RequestSession;
use actix_web::{
    AsyncResponder, Form, FutureResponse, HttpMessage, HttpRequest, HttpResponse, Path, State,
};
use askama::Template;
use bcrypt;
use chrono::Duration;
use chrono_humanize::{Accuracy, HumanTime, Tense};
use data_encoding::BASE64;

use diesel::pg::PgConnection;
use diesel::{self, prelude::*};
use futures::future::{err, ok, result, Either, Future};
use mime_guess::get_mime_type;
use serde::{Deserialize, Serialize};
use std::env;
use uuid::Uuid;

pub mod mfa;
pub mod payment;
pub mod payout;
pub mod webui;

pub fn create_merchant(
    (create_merchant, state): (SimpleJson<CreateMerchant>, State<AppState>),
) -> FutureResponse<HttpResponse> {
    let mut create_merchant = create_merchant.into_inner();
    create_merchant.password = match bcrypt::hash(&create_merchant.password, bcrypt::DEFAULT_COST) {
        Ok(v) => v,
        Err(_) => return result(Ok(HttpResponse::InternalServerError().finish())).responder(),
    };
    state
        .db
        .send(create_merchant)
        .from_err()
        .and_then(|db_response| {
            let merchant = db_response?;
            Ok(HttpResponse::Created().json(merchant))
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

fn check_2fa_code(merchant: &Merchant, code: &str) -> Result<bool, Error> {
    let token_2fa = merchant
        .token_2fa
        .clone()
        .ok_or(Error::General(s!("No 2fa token")))?;
    let totp = Totp::new(merchant.id.clone(), token_2fa);
    Ok(totp.check(code)?)
}

pub trait TemplateIntoResponse {
    fn into_response(&self) -> Result<HttpResponse, Error>;
    fn into_future(&self) -> FutureResponse<HttpResponse, Error>;
}

impl<T: Template> TemplateIntoResponse for T {
    fn into_response(&self) -> Result<HttpResponse, Error> {
        let rsp = self.render().map_err(|e| Error::Template(s!(e)))?;
        let ctype = get_mime_type(T::extension().unwrap_or("txt")).to_string();
        Ok(HttpResponse::Ok().content_type(ctype.as_str()).body(rsp))
    }
    fn into_future(&self) -> FutureResponse<HttpResponse, Error> {
        Box::new(ok(self.into_response().into()))
    }
}

pub trait BootstrapColor {
    fn color(&self) -> &'static str;
}
impl BootstrapColor for Transaction {
    fn color(&self) -> &'static str {
        match (self.transaction_type, self.status) {
            (TransactionType::Payout, TransactionStatus::Confirmed) => "success",
            (TransactionType::Payout, TransactionStatus::Pending) => "info",
            (TransactionType::Payment, TransactionStatus::Rejected) => "secondary",
            (_, _) => "light",
        }
    }
}
