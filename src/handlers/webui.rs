use crate::app::AppState;
use crate::db::{get_balance, GetMerchant};
use crate::errors::*;
use crate::extractor::User;
use crate::filters;
use crate::handlers::paginator::{Pages, Paginate, Paginator};
use crate::handlers::BootstrapColor;
use crate::handlers::TemplateIntoResponse;
use crate::models::{Merchant, Transaction, TransactionStatus, TransactionType};
use actix_session::Session;
use actix_web::middleware::identity::Identity;
use actix_web::web::{block, Data, Form, Query};
use actix_web::{HttpRequest, HttpResponse};
use askama::Template;
use diesel::pg::PgConnection;
use diesel::{self, prelude::*};
use futures::future::Future;
use serde::Deserialize;

#[derive(Template)]
#[template(path = "index.html")]
struct IndexTemplate<'a> {
    merchant: &'a Merchant,
    balance: i64,
    transactions: Vec<Transaction>,
    last_payout: &'a Option<Transaction>,
    current_height: i64,
}

pub fn index(
    merchant: User<Merchant>,
    data: Data<AppState>,
) -> impl Future<Item = HttpResponse, Error = Error> {
    let merchant = merchant.into_inner();
    block::<_, _, Error>({
        let merch_id = merchant.id.clone();
        let pool = data.pool.clone();
        move || {
            let conn: &PgConnection = &pool.get().unwrap();
            let balance = get_balance(&merch_id, conn)?;
            let txs = {
                use crate::schema::transactions::dsl::*;
                transactions
                    .filter(merchant_id.eq(merch_id.clone()))
                    .offset(0)
                    .limit(10)
                    .order(created_at.desc())
                    .load::<Transaction>(conn)
                    .map_err::<Error, _>(|e| e.into())
            }?;
            let last_payout = {
                use crate::schema::transactions::dsl::*;
                transactions
                    .filter(merchant_id.eq(merch_id))
                    .filter(transaction_type.eq(TransactionType::Payout))
                    .order(created_at.desc())
                    .first(conn)
                    .optional()
                    .map_err::<Error, _>(|e| e.into())
            }?;
            let current_height = {
                use crate::schema::current_height::dsl::*;
                current_height
                    .select(height)
                    .first(conn)
                    .map_err::<Error, _>(|e| e.into())
            }?;
            IndexTemplate {
                merchant: &merchant,
                balance: balance,
                transactions: txs,
                last_payout: &last_payout,
                current_height: current_height,
            }
            .render()
            .map_err(|e| Error::from(e))
        }
    })
    .from_err()
    .and_then(move |html| Ok(HttpResponse::Ok().content_type("text/html").body(html)))
}

#[derive(Debug, Deserialize)]
pub struct LoginRequest {
    pub login: String,
    pub password: String,
}
pub fn login(
    data: Data<AppState>,
    login_form: Form<LoginRequest>,
    session: Session,
) -> impl Future<Item = HttpResponse, Error = Error> {
    data.db
        .send(GetMerchant {
            id: login_form.login.clone(),
        })
        .from_err()
        .and_then(move |db_response| {
            let merchant = db_response?;
            match bcrypt::verify(&login_form.password, &merchant.password) {
                Ok(res) => {
                    if res {
                        session.set("merchant", merchant.id)?;
                        if merchant.confirmed_2fa {
                            Ok(HttpResponse::Found().header("location", "/2fa").finish())
                        } else {
                            Ok(HttpResponse::Found()
                                .header("location", "/set_2fa")
                                .finish())
                        }
                    } else {
                        Ok(HttpResponse::Found().header("location", "/login").finish())
                    }
                }
                Err(_) => Ok(HttpResponse::Found().header("location", "/login").finish()),
            }
        })
}

#[derive(Template)]
#[template(path = "login.html")]
struct LoginTemplate;

pub fn login_form(_: HttpRequest) -> Result<HttpResponse, Error> {
    LoginTemplate.into_response()
}

pub fn logout(identity: Identity, session: Session) -> Result<HttpResponse, Error> {
    identity.forget();
    session.clear();
    Ok(HttpResponse::Found().header("location", "/login").finish())
}

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
