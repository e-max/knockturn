use crate::app::AppState;
use crate::errors::*;
use crate::extractor::User;
use crate::filters;
use crate::fsm::{ManuallyRefundPayment, Payment, RefundPayment};
use crate::handlers::paginator::{Pages, Paginate, Paginator};
use crate::handlers::BootstrapColor;
use crate::models::{Merchant, StatusChange, Transaction, TransactionStatus, TransactionType};
use actix_web::web::{block, Data, Path, Query};
use actix_web::HttpResponse;
use askama::Template;
use diesel::pg::PgConnection;
use diesel::{self, prelude::*};
use serde::Deserialize;
use uuid::Uuid;

#[derive(Template)]
#[template(path = "transactions.html")]
struct TransactionsTemplate<'a> {
    transactions: Vec<Transaction>,
    current_height: i64,
    pages: Pages<'a>,
    refunds_num: i64,
    txtype: TxType,
    total: i64,
}

#[derive(Debug, Deserialize)]
pub struct Info {
    txtype: Option<TxType>,
}

#[derive(Debug, Deserialize, Clone, Copy, PartialEq)]
pub enum TxType {
    All,
    Payments,
    Payouts,
    Refunds,
}

impl TxType {
    pub fn is_active(&self, t: &str) -> String {
        match (self, t) {
            (TxType::All, "All") => s!("active"),
            (TxType::Payments, "Payments") => s!("active"),
            (TxType::Payouts, "Payouts") => s!("active"),
            (TxType::Refunds, "Refunds") => s!("active"),
            _ => s!(""),
        }
    }
}

pub async fn get_transactions(
    merchant: User<Merchant>,
    data: Data<AppState>,
    paginate: Paginate,
    info: Query<Info>,
) -> Result<HttpResponse, Error> {
    let merchant = merchant.into_inner();
    let html = block::<_, _, Error>({
        let merch_id = merchant.id.clone();
        let pool = data.pool.clone();
        let paginate = paginate.clone();
        move || {
            use crate::schema::transactions::dsl::*;
            let conn: &PgConnection = &pool.get().unwrap();

            //let txs = transactions
            //.for_page(&paginate)
            //.filter(merchant_id.eq(merch_id.clone()))
            //.order(created_at.desc())
            //.load::<Transaction>(conn)?;

            let txs = match info.txtype.unwrap_or(TxType::All) {
                TxType::Payments => transactions
                    .filter(merchant_id.eq(merch_id.clone()))
                    .filter(transaction_type.eq(TransactionType::Payment))
                    .for_page(&paginate)
                    .order(created_at.desc())
                    .load::<Transaction>(conn),
                TxType::Payouts => transactions
                    .filter(merchant_id.eq(merch_id.clone()))
                    .filter(transaction_type.eq(TransactionType::Payout))
                    .for_page(&paginate)
                    .order(created_at.desc())
                    .load::<Transaction>(conn),
                TxType::Refunds => transactions
                    .filter(merchant_id.eq(merch_id.clone()))
                    .filter(status.eq(TransactionStatus::Refund))
                    .for_page(&paginate)
                    .order(created_at.desc())
                    .load::<Transaction>(conn),
                _ => transactions
                    .filter(merchant_id.eq(merch_id.clone()))
                    .for_page(&paginate)
                    .order(created_at.desc())
                    .load::<Transaction>(conn),
            }?;

            let total = match info.txtype.unwrap_or(TxType::All) {
                TxType::Payments => transactions
                    .filter(merchant_id.eq(merch_id.clone()))
                    .filter(transaction_type.eq(TransactionType::Payment))
                    .count()
                    .first::<i64>(conn),
                TxType::Payouts => transactions
                    .filter(merchant_id.eq(merch_id.clone()))
                    .filter(transaction_type.eq(TransactionType::Payout))
                    .for_page(&paginate)
                    .count()
                    .first::<i64>(conn),
                TxType::Refunds => transactions
                    .filter(merchant_id.eq(merch_id.clone()))
                    .filter(status.eq(TransactionStatus::Refund))
                    .count()
                    .first::<i64>(conn),
                _ => transactions
                    .filter(merchant_id.eq(merch_id.clone()))
                    .count()
                    .first::<i64>(conn),
            }?;

            let refunds = transactions
                .filter(merchant_id.eq(merch_id))
                .filter(status.eq(TransactionStatus::Refund))
                .count()
                .first::<i64>(conn)?;

            let current_height = {
                use crate::schema::current_height::dsl::*;
                current_height
                    .select(height)
                    .first(conn)
                    .map_err::<Error, _>(|e| e.into())
            }?;
            let html = TransactionsTemplate {
                transactions: txs,
                current_height: current_height,
                pages: paginate.for_total(total),
                refunds_num: refunds,
                txtype: info.txtype.clone().unwrap_or(TxType::All),
                total: total,
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
#[template(path = "transaction.html")]
struct TransactionTemplate {
    transaction: Transaction,
    current_height: i64,
}

pub async fn get_transaction(
    merchant: User<Merchant>,
    data: Data<AppState>,
    transaction_id: Path<Uuid>,
) -> Result<HttpResponse, Error> {
    let merchant = merchant.into_inner();
    let html = block::<_, _, Error>({
        let merch_id = merchant.id.clone();
        let pool = data.pool.clone();
        let transaction_id = transaction_id.clone();
        move || {
            use crate::schema::transactions::dsl::*;
            let conn: &PgConnection = &pool.get().unwrap();

            let transaction = transactions
                .filter(id.eq(transaction_id))
                .filter(merchant_id.eq(merch_id.clone()))
                .first::<Transaction>(conn)?;

            let cur_height: i64 = {
                use crate::schema::current_height::dsl::*;
                current_height.select(height).first::<i64>(conn)
            }?;

            let html = TransactionTemplate {
                transaction: transaction,
                current_height: cur_height,
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
#[template(path = "_status_history.html")]
pub struct StatusHistoryTemplate {
    pub history: Vec<StatusChange>,
}

pub async fn get_transaction_status_changes(
    merchant: User<Merchant>,
    transaction_id: Path<Uuid>,
    data: Data<AppState>,
) -> Result<HttpResponse, Error> {
    let merchant = merchant.into_inner();
    let history = block::<_, _, Error>({
        let merch_id = merchant.id.clone();
        let pool = data.pool.clone();
        let tx_id = transaction_id.clone();
        move || {
            let conn: &PgConnection = &pool.get().unwrap();
            let transaction = {
                use crate::schema::transactions::dsl::*;
                transactions
                    .filter(id.eq(tx_id))
                    .filter(merchant_id.eq(merch_id.clone()))
                    .first::<Transaction>(conn)
            }?;
            let history = {
                use crate::schema::status_changes::dsl::*;
                status_changes
                    .filter(transaction_id.eq(transaction.id))
                    .order(updated_at.desc())
                    .load::<StatusChange>(conn)
            }?;
            Ok(history)
        }
    })
    .await?;
    let html = StatusHistoryTemplate { history }
        .render()
        .map_err(|e| Error::from(e))?;
    Ok(HttpResponse::Ok().content_type("text/html").body(html))
}

pub async fn manually_refunded(
    merchant: User<Merchant>,
    data: Data<AppState>,
    transaction_id: Path<Uuid>,
) -> Result<HttpResponse, Error> {
    let merchant_id = merchant.into_inner().id;
    let fsm = data.fsm.clone();

    let payment: RefundPayment = Payment::get(transaction_id.clone(), data.pool.clone()).await?;

    fsm.send(ManuallyRefundPayment {
        payment,
        merchant_id,
    })
    .await??;

    Ok(HttpResponse::Found()
        .header(
            http::header::LOCATION,
            format!("/transactions/{}", transaction_id),
        )
        .finish()
        .into_body())
}
