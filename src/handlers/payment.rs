use crate::app::AppState;
use crate::db::{get_current_height, get_transaction};
use crate::errors::*;
use crate::extractor::{BasicAuth, SimpleJson};
use crate::filters::{self, ForHuman};
use crate::fsm::{CreatePayment, GetNewPayment, MakePayment};
use crate::handlers::BootstrapColor;
use crate::models::{Merchant, Money, Transaction, TransactionStatus};
use crate::qrcode;
use crate::wallet::Slate;
use actix_web::web::{block, Data, Path};
use actix_web::HttpResponse;
use askama::Template;
use data_encoding::BASE64;
use diesel::pg::PgConnection;
use futures::future::Future;
use futures::future::{err, ok, Either};
use serde::{Deserialize, Serialize};
use std::env;
use uuid::Uuid;

#[derive(Debug, Deserialize)]
pub struct CreatePaymentRequest {
    pub order_id: String,
    pub amount: Money,
    pub confirmations: i64,
    pub email: Option<String>,
    pub message: String,
    pub redirect_url: Option<String>,
}

pub fn create_payment(
    merchant: BasicAuth<Merchant>,
    merchant_id: Path<String>,
    payment_req: SimpleJson<CreatePaymentRequest>,
    state: Data<AppState>,
) -> impl Future<Item = HttpResponse, Error = Error> {
    let merchant_id = merchant_id.into_inner();
    if merchant.id != merchant_id {
        return Either::A(err(Error::InvalidEntity(s!("wrong merchant_id"))));
    }
    let create_transaction = CreatePayment {
        merchant_id: merchant_id,
        external_id: payment_req.order_id.clone(),
        amount: payment_req.amount,
        confirmations: payment_req.confirmations,
        email: payment_req.email.clone(),
        message: payment_req.message.clone(),
        redirect_url: payment_req.redirect_url.clone(),
    };
    Either::B(
        state
            .fsm
            .send(create_transaction)
            .from_err()
            .and_then(|db_response| {
                let new_payment = db_response?;

                Ok(HttpResponse::Created().json(new_payment))
            }),
    )
}

#[derive(Debug, Serialize)]
struct PaymentStatus {
    pub transaction_id: String,
    pub status: String,
    pub reported: bool,
    pub seconds_until_expired: Option<i64>,
    pub expired_in: Option<String>,
    pub current_confirmations: i64,
    pub required_confirmations: i64,
}

pub fn get_payment_status(
    transaction_data: Path<(String, Uuid)>,
    state: Data<AppState>,
) -> impl Future<Item = HttpResponse, Error = Error> {
    block::<_, _, Error>({
        let pool = state.pool.clone();
        let (merchant_id, transaction_id) = transaction_data.into_inner();
        move || {
            let conn: &PgConnection = &pool.get().unwrap();
            let current_height = get_current_height(conn)?;
            let tx = get_transaction(transaction_id, conn)?;
            if tx.merchant_id != merchant_id {
                return Err(Error::General(format!("Wrong merchant: {}", merchant_id)));
            }
            let payment_status = PaymentStatus {
                transaction_id: tx.id.to_string(),
                status: tx.status.to_string(),
                seconds_until_expired: tx.time_until_expired().map(|d| d.num_seconds()),

                expired_in: tx.time_until_expired().map(|d| d.for_human()),
                current_confirmations: tx.current_confirmations(current_height),
                required_confirmations: tx.confirmations,
                reported: tx.reported,
            };
            Ok(payment_status)
        }
    })
    .from_err()
    .and_then(|payment_status| Ok(HttpResponse::Ok().json(payment_status)))
}

pub fn get_payment(
    transaction_data: Path<(String, Uuid)>,
    state: Data<AppState>,
) -> impl Future<Item = HttpResponse, Error = Error> {
    block::<_, _, Error>({
        let pool = state.pool.clone();
        let (merchant_id, transaction_id) = transaction_data.into_inner();
        move || {
            let conn: &PgConnection = &pool.get().unwrap();
            let current_height = get_current_height(conn)?;
            let transaction = get_transaction(transaction_id, conn)?;
            if transaction.merchant_id != merchant_id {
                return Err(Error::General(format!("Wrong merchant: {}", merchant_id)));
            }
            let payment_url = format!(
                "{}/merchants/{}/payments/{}",
                env::var("DOMAIN").unwrap().trim_end_matches('/'),
                transaction.merchant_id,
                transaction.id.to_string()
            );
            let ironbelly_link = format!(
                "grin://send?amount={}&destination={}&message={}",
                transaction.grin_amount,
                payment_url,
                BASE64.encode(transaction.message.as_bytes())
            );
            let html = PaymentTemplate {
                payment: &transaction,
                payment_url: payment_url,
                current_height: current_height,
                ironbelly_link: &ironbelly_link,
                ironbelly_qrcode: &BASE64.encode(&qrcode::as_png(&ironbelly_link)?),
            }
            .render()
            .map_err(|e| Error::from(e))?;
            Ok(html)
        }
    })
    .from_err()
    .and_then(|html| Ok(HttpResponse::Ok().content_type("text/html").body(html)))
}

#[derive(Template)]
#[template(path = "payment.html")]
struct PaymentTemplate<'a> {
    payment: &'a Transaction,
    payment_url: String,
    current_height: i64,
    ironbelly_link: &'a str,
    ironbelly_qrcode: &'a str,
}

pub fn make_payment(
    slate: SimpleJson<Slate>,
    payment: Path<GetNewPayment>,
    state: Data<AppState>,
) -> impl Future<Item = HttpResponse, Error = Error> {
    let slate_amount = slate.amount;
    state
        .fsm
        .send(payment.into_inner())
        .from_err()
        .and_then(move |db_response| {
            let new_payment = db_response?;
            let payment_amount = new_payment.grin_amount as u64;
            if new_payment.is_invalid_amount(slate_amount) {
                return Err(Error::WrongAmount(payment_amount, slate_amount));
            }
            Ok(new_payment)
        })
        .and_then({
            let wallet = state.wallet.clone();
            let fsm = state.fsm.clone();
            move |new_payment| {
                let slate = wallet.receive(&slate);
                slate.and_then(move |slate| {
                    let commit = slate.tx.output_commitments()[0].clone();
                    wallet
                        .get_tx(&slate.id.hyphenated().to_string())
                        .and_then(move |wallet_tx| {
                            fsm.send(MakePayment {
                                new_payment,
                                wallet_tx,
                                commit,
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
}
