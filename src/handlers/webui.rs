use crate::app::AppState;
use crate::db::{get_balance, GetMerchant};
use crate::errors::*;
use crate::extractor::User;
use crate::filters;
use crate::handlers::BootstrapColor;
use crate::handlers::TemplateIntoResponse;
use crate::models::{Merchant, Transaction, TransactionType};
use actix_identity::Identity;
use actix_session::Session;
use actix_web::web::{block, Data, Form};
use actix_web::{HttpRequest, HttpResponse};
use askama::Template;
use diesel::pg::PgConnection;
use diesel::{self, prelude::*};
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

pub async fn index(merchant: User<Merchant>, data: Data<AppState>) -> Result<HttpResponse, Error> {
    let merchant = merchant.into_inner();
    let html = block::<_, _, Error>({
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
    .await?;
    Ok(HttpResponse::Ok().content_type("text/html").body(html))
}

#[derive(Debug, Deserialize)]
pub struct LoginRequest {
    pub login: String,
    pub password: String,
}
pub async fn login(
    data: Data<AppState>,
    login_form: Form<LoginRequest>,
    session: Session,
) -> Result<HttpResponse, Error> {
    let merchant = data
        .db
        .send(GetMerchant {
            id: login_form.login.clone(),
        })
        .await??;

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
