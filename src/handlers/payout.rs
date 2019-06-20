use crate::app::AppState;
use crate::errors::*;
use crate::extractor::{SimpleJson, User};
use crate::filters;
use crate::fsm::MINIMAL_WITHDRAW;
use crate::fsm_payout::{
    CreatePayout, FinalizePayout, GetInitializedPayout, GetNewPayout, GetPayout, InitializePayout,
    PayoutFees,
};
use crate::handlers::check_2fa_code;
use crate::handlers::BootstrapColor;
use crate::handlers::TemplateIntoResponse;
use crate::models::{Merchant, Money, Transaction, TransactionStatus};
use crate::wallet::Slate;
use actix_web::middleware::identity::Identity;
use actix_web::web::{Data, Form, Path};
use actix_web::{HttpRequest, HttpResponse};

use askama::Template;
use futures::future::{ok, result, Either, Future};
use serde::Deserialize;
use uuid::Uuid;

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

pub fn withdraw(merchant: User<Merchant>) -> Result<HttpResponse, Error> {
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

    template.into_response()
}

#[derive(Debug, Deserialize)]
pub struct CreatePayoutForm {
    pub amount: i64,
    pub code: String,
}

pub fn create_payout(
    data: Data<AppState>,
    form: Form<CreatePayoutForm>,
    identity_merchant: User<Merchant>,
) -> impl Future<Item = HttpResponse, Error = Error> {
    let merchant = identity_merchant.clone().into_inner();
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
                Either::A(result(withdraw(identity_merchant)))
            } else {
                Either::B(
                    data.fsm_payout
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
                        }),
                )
            }
        })
}

#[derive(Template)]
#[template(path = "payout.html")]
struct PayoutTemplate<'a> {
    payout: &'a Transaction,
}

pub fn get_payout(
    transaction_id: Path<Uuid>,
    state: Data<AppState>,
    id: Identity,
) -> impl Future<Item = HttpResponse, Error = Error> {
    let merchant_id = id.identity().unwrap();

    state
        .fsm_payout
        .send(GetPayout {
            merchant_id: merchant_id,
            transaction_id: transaction_id.clone(),
        })
        .from_err()
        .and_then(|db_response| {
            let transaction = db_response?;
            let _knockturn_fee = transaction
                .knockturn_fee
                .ok_or(Error::General(s!("Transaction doesn't have knockturn_fee")))?;
            let _transfer_fee = transaction
                .transfer_fee
                .ok_or(Error::General(s!("Transaction doesn't have transfer_fee")))?;
            let html = PayoutTemplate {
                payout: &transaction,
            }
            .render()
            .map_err(|e| Error::from(e))?;
            Ok(HttpResponse::Ok().content_type("text/html").body(html))
        })
}

pub fn generate_slate(
    transaction_id: Path<Uuid>,
    state: Data<AppState>,
    id: Identity,
) -> impl Future<Item = HttpResponse, Error = Error> {
    let merchant_id = match id.identity() {
        Some(v) => v,
        None => {
            return Either::A(ok(HttpResponse::Found()
                .header("location", "/login")
                .finish()))
        }
    };

    Either::B(
        state
            .fsm_payout
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
                let fsm = state.fsm_payout.clone();
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
            }),
    )
}

pub fn accept_slate(
    slate: SimpleJson<Slate>,
    tx_id: Path<Uuid>,
    state: Data<AppState>,
) -> impl Future<Item = HttpResponse, Error = Error> {
    state
        .fsm_payout
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
            let fsm = state.fsm_payout.clone();
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
}

pub fn withdraw_confirmation(_req: HttpRequest) -> Result<HttpResponse, Error> {
    Ok(HttpResponse::Ok().body("hello"))
}
