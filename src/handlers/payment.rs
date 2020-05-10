use crate::app::AppState;
use crate::db::{get_current_height, get_transaction};
use crate::errors::*;
use crate::extractor::{BasicAuth, SimpleJson};
use crate::filters::{self, ForHuman};
use crate::fsm::{CreatePayment, MakePayment, NewPayment, Payment};
use crate::handlers::BootstrapColor;
use crate::jsonrpc;
use crate::models::{Merchant, Money, Transaction, TransactionStatus};
use crate::qrcode;
use crate::wallet::Slate;
use actix_web::web::{block, Data, Path};
use actix_web::HttpResponse;
use askama::Template;
use data_encoding::BASE64;
use diesel::pg::PgConnection;
use log::*;
use serde::{Deserialize, Serialize};
use serde_json;
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

pub async fn create_payment(
    merchant: BasicAuth<Merchant>,
    merchant_id: Path<String>,
    payment_req: SimpleJson<CreatePaymentRequest>,
    state: Data<AppState>,
) -> Result<HttpResponse, Error> {
    let merchant_id = merchant_id.into_inner();
    if merchant.id != merchant_id {
        return Err(Error::InvalidEntity(s!("wrong merchant_id")));
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
    let new_payment = state.fsm.send(create_transaction).await??;
    Ok(HttpResponse::Created().json(new_payment))
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

pub async fn get_payment_status(
    transaction_data: Path<(String, Uuid)>,
    state: Data<AppState>,
) -> Result<HttpResponse, Error> {
    let payment_status = block::<_, _, Error>({
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
    .await?;
    Ok(HttpResponse::Ok().json(payment_status))
}

pub async fn get_payment(
    transaction_data: Path<(String, Uuid)>,
    state: Data<AppState>,
) -> Result<HttpResponse, Error> {
    let html = block::<_, _, Error>({
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
    .await?;
    Ok(HttpResponse::Ok().content_type("text/html").body(html))
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

//quick and dirty implementaion of JSONRPC for wallet client
pub async fn wallet_jsonrpc(
    req: jsonrpc::Request,
    payment_data: Path<(String, Uuid)>,
    state: Data<AppState>,
) -> Result<HttpResponse, ()> {
    let (merchant_id, payment_id) = payment_data.into_inner();
    let req_id = req.id.clone();

    let res: Result<jsonrpc::Response, jsonrpc::ErrorData> = match req.method.as_ref() {
        "receive_tx" => {
            let params = req.params.clone();
            if params.len() != 3 {
                Err(jsonrpc::ErrorData::std(-32602))
            } else {
                match serde_json::from_value(params[0].clone()) {
                    Ok(slate) => pay_slate2(req, slate, merchant_id, payment_id, state)
                        .await
                        .map_err(|e| {
                            error!("Error in pay_slate {}", e);
                            jsonrpc::ErrorData {
                                code: 32000,
                                message: format!("{}", e),
                                data: serde_json::Value::Null,
                            }
                        }),
                    Err(e) => {
                        error!("Cannot parse slate: {}", e);
                        Err(jsonrpc::ErrorData::std(-32602))
                    }
                }
            }
        }
        _ => state.wallet.jsonrpc_request(req, false).await.map_err(|e| {
            error!("Error while proxying request {}", e);
            jsonrpc::ErrorData {
                code: 32000,
                message: format!("{}", e),
                data: serde_json::Value::Null,
            }
        }),
    };
    res.or_else({
        let req_id = req_id.clone();
        |e| {
            error!("Got jsonrpc error {}", e);
            let mut resp = jsonrpc::Response::with_id(req_id);
            resp.error = Some(e);
            Ok(resp)
        }
    })
    .and_then(|resp| {
        Ok(HttpResponse::Ok()
            .content_type("application/json")
            .body(resp.dump()))
    })
    .map_err(|e: jsonrpc::ErrorData| ()) //ignore an error because we converted it to response anyway
}

#[derive(Debug, Serialize)]
pub struct APIVersion {
    foreign_api_version: u16,
    supported_slate_versions: Vec<String>,
}
impl Default for APIVersion {
    fn default() -> Self {
        APIVersion {
            foreign_api_version: 2,
            supported_slate_versions: vec![s!("V1"), s!("V2")],
        }
    }
}

pub fn check_version() -> APIVersion {
    APIVersion::default()
}

pub async fn pay_slate2(
    req: jsonrpc::Request,
    slate: Slate,
    merchant_id: String,
    payment_id: Uuid,
    state: Data<AppState>,
) -> Result<jsonrpc::Response, Error> {
    let slate_amount = slate.amount;

    let new_payment: NewPayment = Payment::get(payment_id, state.pool.clone()).await?;
    let payment_amount = new_payment.grin_amount as u64;
    if new_payment.is_invalid_amount(slate_amount) {
        return Err(Error::WrongAmount(payment_amount, slate_amount));
    }
    let wallet = state.wallet.clone();
    let fsm = state.fsm.clone();

    let resp = wallet.jsonrpc_request(req, false).await?;
    let commit = slate.tx.output_commitments()[0].clone();
    let wallet_tx = wallet.get_tx(&slate.id.hyphenated().to_string()).await?;
    fsm.send(MakePayment {
        new_payment,
        wallet_tx,
        commit,
    })
    .await??;
    Ok(resp)
}

pub async fn make_payment(
    slate: SimpleJson<Slate>,
    payment_data: Path<(String, Uuid)>,
    state: Data<AppState>,
) -> Result<HttpResponse, Error> {
    let (merchant_id, payment_id) = payment_data.into_inner();

    let slate_amount = slate.amount;

    let new_payment: NewPayment = Payment::get(payment_id, state.pool.clone()).await?;
    let payment_amount = new_payment.grin_amount as u64;
    if new_payment.is_invalid_amount(slate_amount) {
        return Err(Error::WrongAmount(payment_amount, slate_amount));
    }
    let wallet = state.wallet.clone();
    let fsm = state.fsm.clone();
    let slate = wallet.receive(&slate).await?;
    let commit = slate.tx.output_commitments()[0].clone();
    let wallet_tx = wallet.get_tx(&slate.id.hyphenated().to_string()).await?;

    fsm.send(MakePayment {
        new_payment,
        wallet_tx,
        commit,
    })
    .await??;

    Ok(HttpResponse::Ok().json(slate))
}
