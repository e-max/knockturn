use crate::app::AppState;
use crate::errors::*;
use crate::extractor::User;
use crate::filters;
use crate::handlers::paginator::{Pages, Paginate, Paginator};
use crate::handlers::BootstrapColor;
use crate::models::{Merchant, Transaction, TransactionStatus, TransactionType};
use actix_web::web::{block, Data, Form, Query};
use actix_web::{HttpRequest, HttpResponse};
use askama::Template;
use diesel::pg::PgConnection;
use diesel::{self, prelude::*};
use futures::future::Future;
use serde::Deserialize;

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

pub fn get_transactions(
    merchant: User<Merchant>,
    data: Data<AppState>,
    paginate: Paginate,
    info: Query<Info>,
) -> impl Future<Item = HttpResponse, Error = Error> {
    let merchant = merchant.into_inner();
    block::<_, _, Error>({
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
    .from_err()
    .and_then(move |html| Ok(HttpResponse::Ok().content_type("text/html").body(html)))
}
