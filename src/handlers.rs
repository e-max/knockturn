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

#[derive(Template, Debug)]
#[template(path = "withdraw.html")]
struct WithdrawTemplate<'a> {
    error: Option<String>,
    balance: Money,
    knockturn_fee: Money,
    transfer_fee: Money,
    total: Money,
    url: &'a str,
}

pub fn withdraw(
    (merchant, req): (Identity<Merchant>, HttpRequest<AppState>),
) -> FutureResponse<HttpResponse, Error> {
    let reminder = merchant.balance.reminder().unwrap_or(0);
    let mut template = WithdrawTemplate {
        error: None,
        balance: merchant.balance.into(),
        transfer_fee: merchant.balance.transfer_fee().into(),
        knockturn_fee: merchant.balance.knockturn_fee().into(),
        total: reminder.into(),
        url: "http://localhost:6000/withdraw/confirm",
    };

    if merchant.balance < MINIMAL_WITHDRAW {
        template.error = Some(format!(
            "You balance is too small. Minimal withdraw amount is {}",
            Money::from_grin(MINIMAL_WITHDRAW)
        ));
    }

    template.into_future()
}

fn check_2fa_code(merchant: &Merchant, code: &str) -> Result<bool, Error> {
    let token_2fa = merchant
        .token_2fa
        .clone()
        .ok_or(Error::General(s!("No 2fa token")))?;
    let totp = Totp::new(merchant.id.clone(), token_2fa);
    Ok(totp.check(code)?)
}

#[derive(Debug, Deserialize)]
pub struct CreatePayoutForm {
    pub amount: i64,
    pub code: String,
}

pub fn create_payout(
    (req, form, identity_merchant): (
        HttpRequest<AppState>,
        Form<CreatePayoutForm>,
        Identity<Merchant>,
    ),
) -> FutureResponse<HttpResponse, Error> {
    let merchant = identity_merchant.clone().into_inner();
    Box::new(
        ok(())
            .and_then({
                let code = form.code.clone();
                move |_| {
                    let validated = check_2fa_code(&merchant, &code)?;
                    Ok((merchant, validated))
                }
            })
            .and_then(move |(merchant, validated)| {
                if !validated {
                    withdraw((identity_merchant, req))
                } else {
                    req.state()
                        .fsm
                        .send(CreatePayout {
                            amount: form.amount,
                            merchant_id: merchant.id,
                            confirmations: 10,
                        })
                        .from_err()
                        .and_then(|resp| {
                            let payout = resp?;
                            Ok(HttpResponse::Found()
                                .header("location", format!("/payouts/{}", payout.id))
                                .finish())
                        })
                        .responder()
                }
            }),
    )
}

#[derive(Template)]
#[template(path = "payout.html")]
struct PayoutTemplate<'a> {
    payout: &'a Transaction,
}

pub fn get_payout(
    (req, transaction_id, state): (HttpRequest<AppState>, Path<Uuid>, State<AppState>),
) -> FutureResponse<HttpResponse> {
    let merchant_id = req.identity().unwrap();

    state
        .fsm
        .send(GetPayout {
            merchant_id: merchant_id,
            transaction_id: transaction_id.clone(),
        })
        .from_err()
        .and_then(|db_response| {
            let transaction = db_response?;
            let knockturn_fee = transaction
                .knockturn_fee
                .ok_or(Error::General(s!("Transaction doesn't have knockturn_fee")))?;
            let transfer_fee = transaction
                .transfer_fee
                .ok_or(Error::General(s!("Transaction doesn't have transfer_fee")))?;
            let html = PayoutTemplate {
                payout: &transaction,
            }
            .render()
            .map_err(|e| Error::from(e))?;
            Ok(HttpResponse::Ok().content_type("text/html").body(html))
        })
        .responder()
}

pub fn generate_slate(
    (req, transaction_id, state): (HttpRequest<AppState>, Path<Uuid>, State<AppState>),
) -> FutureResponse<HttpResponse, Error> {
    let merchant_id = match req.identity() {
        Some(v) => v,
        None => return ok(HttpResponse::Found().header("location", "/login").finish()).responder(),
    };

    let res = state
        .fsm
        .send(GetNewPayout {
            merchant_id: merchant_id,
            transaction_id: transaction_id.clone(),
        })
        .from_err()
        .and_then(|db_response| {
            let payout = db_response?;
            Ok(payout)
        })
        .and_then({
            let wallet = state.wallet.clone();
            move |new_payout| {
                let real_payment = new_payout.grin_amount
                    - new_payout.transfer_fee.unwrap()
                    - new_payout.knockturn_fee.unwrap();
                wallet
                    .create_slate(real_payment as u64, new_payout.message.clone())
                    .and_then(move |slate| Ok((new_payout, slate)))
            }
        })
        .and_then({
            let wallet = state.wallet.clone();
            move |(new_payout, slate)| {
                wallet
                    .get_tx(&slate.id.hyphenated().to_string())
                    .and_then(|wallet_tx| Ok((new_payout, slate, wallet_tx)))
            }
        })
        .and_then({
            let fsm = state.fsm.clone();
            move |(new_payout, slate, wallet_tx)| {
                let commit = slate.tx.output_commitments()[0].clone();
                fsm.send(InitializePayout {
                    new_payout,
                    wallet_tx,
                    commit,
                })
                .from_err()
                .and_then(|db_response| {
                    db_response?;
                    Ok(())
                })
                .and_then(|_| ok(slate))
            }
        })
        .and_then(|slate| {
            Ok(HttpResponse::Ok()
                .content_type("application/octet-stream")
                .json(slate))
        });

    Box::new(res)
}

pub fn accept_slate(
    (slate, tx_id, req, state): (
        SimpleJson<Slate>,
        Path<Uuid>,
        HttpRequest<AppState>,
        State<AppState>,
    ),
) -> FutureResponse<HttpResponse, Error> {
    let slate_amount = slate.amount;
    state
        .fsm
        .send(GetInitializedPayout {
            transaction_id: tx_id.clone(),
        })
        .from_err()
        .and_then(move |db_response| {
            let initialized_payout = db_response?;
            Ok(initialized_payout)
        })
        .and_then({
            let wallet = state.wallet.clone();
            let fsm = state.fsm.clone();
            move |initialized_payout| {
                wallet.finalize(&slate).and_then({
                    let wallet = wallet.clone();
                    move |slate| {
                        wallet
                            .post_tx()
                            .and_then(move |_| {
                                fsm.send(FinalizePayout { initialized_payout })
                                    .from_err()
                                    .and_then(|db_response| {
                                        db_response?;
                                        Ok(())
                                    })
                            })
                            .and_then(|_| ok(slate))
                    }
                })
            }
        })
        .and_then(|slate| Ok(HttpResponse::Ok().json(slate)))
        .responder()
}

pub fn withdraw_confirmation(
    (slate, req, state): (SimpleJson<Slate>, HttpRequest<AppState>, State<AppState>),
) -> Result<HttpResponse, Error> {
    Ok(HttpResponse::Ok().body("hello"))
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
