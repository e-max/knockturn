use crate::app::AppState;
use crate::blocking;
use crate::db::{Confirm2FA, CreateMerchant, GetMerchant, GetTransaction, GetTransactions};
use crate::errors::*;
use crate::fsm::{CreatePayment, GetUnpaidPayment, MakePayment};
use crate::middleware::WithMerchant;
use crate::models::{Currency, Money, Transaction};
use crate::totp::Totp;
use crate::wallet::Slate;
use actix_web::http::Method;
use actix_web::middleware::identity::RequestIdentity;
use actix_web::middleware::session::RequestSession;
use actix_web::{
    AsyncResponder, Form, FutureResponse, HttpMessage, HttpRequest, HttpResponse, Json, Path, State,
};
use actix_web_httpauth::extractors::basic::BasicAuth;
use askama::Template;
use bcrypt;
use bytes::BytesMut;
use data_encoding::BASE64;
use diesel::pg::PgConnection;
use diesel::r2d2::{ConnectionManager, Pool};
use diesel::{self, prelude::*};
use futures::future::{ok, result, Either, Future};
use futures::stream::Stream;
use mime_guess::get_mime_type;
use serde::Deserialize;
use std::env;

#[derive(Template)]
#[template(path = "index.html")]
struct IndexTemplate<'a> {
    merchant_id: &'a str,
    transactions: Vec<Transaction>,
}

pub fn index(req: &HttpRequest<AppState>) -> FutureResponse<HttpResponse> {
    let merchant_id = match req.identity() {
        Some(v) => v,
        None => return ok(HttpResponse::Found().header("location", "/login").finish()).responder(),
    };
    req.state()
        .db
        .send(GetTransactions {
            merchant_id: merchant_id.clone(),
            offset: 0,
            limit: 100,
        })
        .from_err()
        .and_then(move |db_response| {
            let transactions = db_response?;
            let html = IndexTemplate {
                merchant_id: &merchant_id,
                transactions,
            }
            .render()
            .map_err(|e| Error::from(e))?;
            Ok(HttpResponse::Ok().content_type("text/html").body(html))
        })
        .responder()
}

#[derive(Template)]
#[template(path = "transactions.html")]
struct TransactionsTemplate {
    transactions: Vec<Transaction>,
}

pub fn get_transactions(req: &HttpRequest<AppState>) -> FutureResponse<HttpResponse> {
    let merchant_id = match req.identity() {
        Some(v) => v,
        None => return ok(HttpResponse::Found().header("location", "/login").finish()).responder(),
    };

    blocking::run({
        let merch_id = merchant_id.clone();
        let pool = req.state().pool.clone();
        move || {
            use crate::schema::transactions::dsl::*;
            let conn: &PgConnection = &pool.get().unwrap();
            transactions
                .filter(merchant_id.eq(merch_id))
                .offset(0)
                .limit(10)
                .load::<Transaction>(conn)
                .map_err(|e| e.into())
        }
    })
    .from_err()
    .and_then(|transactions| {
        let html = TransactionsTemplate { transactions }
            .render()
            .map_err(|e| Error::from(e))?;
        Ok(HttpResponse::Ok().content_type("text/html").body(html))
    })
    .responder()
}

#[derive(Debug, Deserialize)]
pub struct LoginRequest {
    pub login: String,
    pub password: String,
}
pub fn login(
    (req, login_form): (HttpRequest<AppState>, Form<LoginRequest>),
) -> FutureResponse<HttpResponse> {
    req.state()
        .db
        .send(GetMerchant {
            id: login_form.login.clone(),
        })
        .from_err()
        .and_then(move |db_response| {
            let merchant = db_response?;
            match bcrypt::verify(&login_form.password, &merchant.password) {
                Ok(res) => {
                    if res {
                        req.session().set("merchant", merchant.id)?;
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
        .responder()
}

#[derive(Template)]
#[template(path = "login.html")]
struct LoginTemplate;

pub fn login_form(_: HttpRequest<AppState>) -> Result<HttpResponse, Error> {
    LoginTemplate.into_response()
}

pub fn logout(req: HttpRequest<AppState>) -> Result<HttpResponse, Error> {
    req.forget();
    req.session().clear();
    Ok(HttpResponse::Found().header("location", "/login").finish())
}

pub fn create_merchant(
    (mut create_merchant, state): (Json<CreateMerchant>, State<AppState>),
) -> FutureResponse<HttpResponse> {
    create_merchant.password = match bcrypt::hash(&create_merchant.password, bcrypt::DEFAULT_COST) {
        Ok(v) => v,
        Err(_) => return result(Ok(HttpResponse::InternalServerError().finish())).responder(),
    };
    state
        .db
        .send(create_merchant.into_inner())
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

#[derive(Debug, Deserialize)]
pub struct CreatePaymentRequest {
    pub order_id: String,
    pub amount: Money,
    pub confirmations: i32,
    pub email: Option<String>,
    pub message: String,
}

pub fn create_payment(
    (merchant_id, payment_req, state): (Path<String>, Json<CreatePaymentRequest>, State<AppState>),
) -> FutureResponse<HttpResponse> {
    let create_transaction = CreatePayment {
        merchant_id: merchant_id.into_inner(),
        external_id: payment_req.order_id.clone(),
        amount: payment_req.amount,
        confirmations: payment_req.confirmations,
        email: payment_req.email.clone(),
        message: payment_req.message.clone(),
    };
    state
        .fsm
        .send(create_transaction)
        .from_err()
        .and_then(|db_response| {
            let unpaid_payment = db_response?;

            Ok(HttpResponse::Created().json(unpaid_payment))
        })
        .responder()
}

pub fn get_payment(
    (get_transaction, state): (Path<GetTransaction>, State<AppState>),
) -> FutureResponse<HttpResponse> {
    state
        .db
        .send(get_transaction.into_inner())
        .from_err()
        .and_then(|db_response| {
            let transaction = db_response?;
            let html = PaymentTemplate {
                transaction_id: transaction.id.to_string(),
                merchant_id: transaction.merchant_id.clone(),
                amount: transaction.amount,
                grin_amount: Money::new(transaction.grin_amount, Currency::GRIN),
                status: transaction.status.to_string(),
                payment_url: format!(
                    "{}/merchants/{}/payments/{}",
                    env::var("DOMAIN").unwrap().trim_end_matches('/'),
                    transaction.merchant_id,
                    transaction.id.to_string()
                ),
            }
            .render()
            .map_err(|e| Error::from(e))?;
            Ok(HttpResponse::Ok().content_type("text/html").body(html))
        })
        .responder()
}

#[derive(Template)]
#[template(path = "payment.html")]
struct PaymentTemplate {
    transaction_id: String,
    merchant_id: String,
    status: String,
    amount: Money,
    grin_amount: Money,
    payment_url: String,
}

pub fn get_tx(state: State<AppState>) -> Box<Future<Item = HttpResponse, Error = Error>> {
    state
        .wallet
        .get_tx("c3b4be4a-b72c-46f5-8fb0-e318ca19ba2b")
        .and_then(|body| Ok(HttpResponse::Ok().json(body)))
        .responder()
}

const MAX_SIZE: usize = 262_144; // max payload size is 256k

#[derive(Template)]
#[template(path = "totp.html")]
struct TotpTemplate<'a> {
    msg: &'a str,
    token: &'a str,
    image: &'a str,
}

#[derive(Debug, Deserialize)]
pub struct TotpRequest {
    pub code: String,
}

pub fn get_totp(req: HttpRequest<AppState>) -> Result<HttpResponse, Error> {
    let merchant = req.get_merchant().ok_or(Error::MerchantNotFound)?;
    let token = merchant
        .token_2fa
        .ok_or(Error::General(s!("No 2fa token")))?;
    let totp = Totp::new(merchant.id.clone(), token.clone());

    let html = TotpTemplate {
        msg: "",
        token: &token,
        image: &BASE64.encode(&totp.get_png()?),
    }
    .render()
    .map_err(|e| Error::from(e))?;
    Ok(HttpResponse::Ok().content_type("text/html").body(html))
}

pub fn post_totp(
    (req, totp_form): (HttpRequest<AppState>, Form<TotpRequest>),
) -> FutureResponse<HttpResponse, Error> {
    let merchant_id = match req.session().get::<String>("merchant") {
        Ok(Some(v)) => v,
        _ => {
            return Box::new(ok(HttpResponse::Found()
                .header("location", "/login")
                .finish()));
        }
    };
    req.state()
        .db
        .send(GetMerchant {
            id: merchant_id.clone(),
        })
        .from_err()
        .and_then({
            let request_method = req.method().clone();
            move |db_response| {
                let merchant = db_response?;

                let mut msg = String::new();

                let token = merchant
                    .token_2fa
                    .ok_or(Error::General(s!("No 2fa token")))?;
                let totp = Totp::new(merchant.id.clone(), token.clone());

                if request_method == Method::POST {
                    if totp.check(&totp_form.code)? {
                        let resp = HttpResponse::Found().header("location", "/").finish();
                        return Ok((true, resp));
                    }
                    msg.push_str("Incorrect code, please try one more time");
                }

                let html = TotpTemplate {
                    msg: &msg,
                    token: &token,
                    image: &BASE64.encode(&totp.get_png()?),
                }
                .render()
                .map_err(|e| Error::from(e))?;
                let resp = HttpResponse::Ok().content_type("text/html").body(html);
                Ok((false, resp))
            }
        })
        .and_then({
            let db = req.state().db.clone();
            move |(confirm, response)| {
                if confirm {
                    Either::A(db.send(Confirm2FA { merchant_id }).from_err().and_then(
                        move |db_response| {
                            db_response?;
                            Ok(response)
                        },
                    ))
                } else {
                    Either::B(ok(response))
                }
            }
        })
        .responder()
}

pub fn make_payment(
    (payment, req, state): (
        Path<GetUnpaidPayment>,
        HttpRequest<AppState>,
        State<AppState>,
    ),
) -> FutureResponse<HttpResponse, Error> {
    req.payload()
        .map_err(|e| Error::Internal(format!("Payload error: {:?}", e)))
        //.from_err()
        .fold(BytesMut::new(), move |mut body, chunk| {
            if (body.len() + chunk.len()) > MAX_SIZE {
                Err(Error::Internal("overflow".to_owned()))
            } else {
                body.extend_from_slice(&chunk);
                Ok(body)
            }
        })
        .and_then(|body| {
            let slate = serde_json::from_slice::<Slate>(&body)?;
            Ok(slate)
        })
        .and_then(move |slate| {
            let slate_amount = slate.amount;
            state
                .fsm
                .send(payment.into_inner())
                .from_err()
                .and_then(move |db_response| {
                    let unpaid_payment = db_response?;
                    if unpaid_payment.grin_amount != slate_amount as i64 {
                        return Err(Error::WrongAmount(
                            unpaid_payment.grin_amount as u64,
                            slate_amount,
                        ));
                    }
                    Ok(unpaid_payment)
                })
                .and_then({
                    let wallet = state.wallet.clone();
                    let fsm = state.fsm.clone();
                    move |unpaid_payment| {
                        let slate = wallet.receive(&slate);
                        slate.and_then(move |slate| {
                            wallet
                                .get_tx(&slate.id.hyphenated().to_string())
                                .and_then(move |wallet_tx| {
                                    fsm.send(MakePayment {
                                        unpaid_payment,
                                        wallet_tx,
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
        })
        .responder()
}

#[derive(Template)]
#[template(path = "2fa.html")]
struct TwoFATemplate;

pub fn form_2fa(_: HttpRequest<AppState>) -> Result<HttpResponse, Error> {
    TwoFATemplate {}.into_response()
}

pub fn post_2fa(
    (req, totp_form): (HttpRequest<AppState>, Form<TotpRequest>),
) -> FutureResponse<HttpResponse, Error> {
    let merchant_id = match req.session().get::<String>("merchant") {
        Ok(Some(v)) => v,
        _ => {
            return Box::new(ok(HttpResponse::Found()
                .header("location", "/login")
                .finish()));
        }
    };
    req.state()
        .db
        .send(GetMerchant {
            id: merchant_id.clone(),
        })
        .from_err()
        .and_then(move |db_response| {
            let merchant = db_response?;

            let token = merchant
                .token_2fa
                .ok_or(Error::General(s!("No 2fa token")))?;
            let totp = Totp::new(merchant.id.clone(), token.clone());

            if totp.check(&totp_form.code)? {
                req.remember(merchant.id);
                return Ok(HttpResponse::Found().header("location", "/").finish());
            } else {
                Ok(HttpResponse::Found().header("location", "/2fa").finish())
            }
        })
        .responder()
}

pub trait TemplateIntoResponse {
    fn into_response(&self) -> Result<HttpResponse, Error>;
}

impl<T: Template> TemplateIntoResponse for T {
    fn into_response(&self) -> Result<HttpResponse, Error> {
        let rsp = self.render().map_err(|e| Error::Template(s!(e)))?;
        let ctype = get_mime_type(T::extension().unwrap_or("txt")).to_string();
        Ok(HttpResponse::Ok().content_type(ctype.as_str()).body(rsp))
    }
}
