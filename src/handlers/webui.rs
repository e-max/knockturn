use crate::app::AppState;
use crate::db::GetMerchant;
use crate::errors::*;
use crate::extractor::User;
use crate::filters;
use crate::handlers::paginator::{Pages, Paginate, Paginator};
use crate::handlers::BootstrapColor;
use crate::handlers::TemplateIntoResponse;
use crate::models::{Merchant, Transaction, TransactionType};
use actix_session::Session;
use actix_web::middleware::identity::Identity;
use actix_web::web::{block, Data, Form};
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
            Ok((txs, last_payout, current_height))
        }
    })
    .from_err()
    .and_then(move |(transactions, last_payout, current_height)| {
        let html = IndexTemplate {
            merchant: &merchant,
            transactions: transactions,
            last_payout: &last_payout,
            current_height: current_height,
        }
        .render()
        .map_err(|e| Error::from(e))?;
        Ok(HttpResponse::Ok().content_type("text/html").body(html))
    })
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
}

pub fn get_transactions(
    merchant: User<Merchant>,
    data: Data<AppState>,
    paginate: Paginate,
) -> impl Future<Item = HttpResponse, Error = Error> {
    let merchant = merchant.into_inner();
    block::<_, _, Error>({
        let merch_id = merchant.id.clone();
        let pool = data.pool.clone();
        let paginate = paginate.clone();
        move || {
            use crate::schema::transactions::dsl::*;
            let conn: &PgConnection = &pool.get().unwrap();
            let txs = transactions
                .for_page(&paginate)
                .filter(merchant_id.eq(merch_id.clone()))
                .load::<Transaction>(conn)
                .map_err::<Error, _>(|e| e.into())?;

            let total = transactions
                .filter(merchant_id.eq(merch_id))
                .count()
                .first(conn)?;

            let current_height = {
                use crate::schema::current_height::dsl::*;
                current_height
                    .select(height)
                    .first(conn)
                    .map_err::<Error, _>(|e| e.into())
            }?;
            Ok((txs, current_height, total))
        }
    })
    .from_err()
    .and_then(move |(transactions, current_height, total)| {
        let html = TransactionsTemplate {
            transactions: transactions,
            current_height: current_height,
            pages: paginate.for_total(total),
        }
        .render()
        .map_err(|e| Error::from(e))?;
        Ok(HttpResponse::Ok().content_type("text/html").body(html))
    })
}
