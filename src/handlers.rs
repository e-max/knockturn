use crate::app::AppState;
use crate::blocking;
use crate::db::{Confirm2FA, CreateMerchant, GetMerchant, GetTransaction, GetTransactions};
use crate::errors::*;
use crate::fsm::{
    CreatePayment, CreatePayout, FinalizePayout, GetInitializedPayout, GetNewPayment, GetNewPayout,
    GetPayout, InitializePayout, MakePayment,
};
use crate::fsm::{KNOCKTURN_SHARE, MINIMAL_WITHDRAW, TRANSFER_FEE};
use crate::middleware::WithMerchant;
use crate::models::{Currency, Money, Transaction, TransactionStatus};
use crate::totp::Totp;
use crate::wallet::Slate;
use actix_web::http::Method;
use actix_web::middleware::identity::RequestIdentity;
use actix_web::middleware::session::RequestSession;
use actix_web::{
    AsyncResponder, Form, FutureResponse, HttpMessage, HttpRequest, HttpResponse, Json, Path, State,
};
use askama::Template;
use bcrypt;
use bytes::BytesMut;
use chrono::Duration;
use data_encoding::BASE64;
use diesel::pg::PgConnection;
use diesel::{self, prelude::*};
use futures::future::{ok, result, Either, Future};
use futures::stream::Stream;
use mime_guess::get_mime_type;
use serde::{Deserialize, Serialize};
use std::env;
use uuid::Uuid;

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
            let new_payment = db_response?;

            Ok(HttpResponse::Created().json(new_payment))
        })
        .responder()
}

#[derive(Debug, Serialize)]
struct PaymentStatus {
    pub transaction_id: String,
    pub status: String,
    pub seconds_until_expired: Option<i64>,
}

pub fn get_payment_status(
    (get_transaction, state): (Path<GetTransaction>, State<AppState>),
) -> FutureResponse<HttpResponse> {
    state
        .db
        .send(get_transaction.into_inner())
        .from_err()
        .and_then(|db_response| {
            let tx = db_response?;
            let payment_status = PaymentStatus {
                transaction_id: tx.id.to_string(),
                status: tx.status.to_string(),
                seconds_until_expired: tx.time_until_expired().map(|d| d.num_seconds()),
            };
            Ok(HttpResponse::Ok().json(payment_status))
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
                time_until_expired: transaction.time_until_expired(),
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
    time_until_expired: Option<Duration>,
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
    (payment, req, state): (Path<GetNewPayment>, HttpRequest<AppState>, State<AppState>),
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
                    let new_payment = db_response?;
                    if new_payment.grin_amount != slate_amount as i64 {
                        return Err(Error::WrongAmount(
                            new_payment.grin_amount as u64,
                            slate_amount,
                        ));
                    }
                    Ok(new_payment)
                })
                .and_then({
                    let wallet = state.wallet.clone();
                    let fsm = state.fsm.clone();
                    move |new_payment| {
                        let slate = wallet.receive(&slate);
                        slate.and_then(move |slate| {
                            wallet
                                .get_tx(&slate.id.hyphenated().to_string())
                                .and_then(move |wallet_tx| {
                                    fsm.send(MakePayment {
                                        new_payment,
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

pub fn withdraw(req: HttpRequest<AppState>) -> FutureResponse<HttpResponse, Error> {
    let merchant_id = match req.identity() {
        Some(v) => v,
        None => return ok(HttpResponse::Found().header("location", "/login").finish()).responder(),
    };
    Box::new(
        req.state()
            .db
            .send(GetMerchant {
                id: merchant_id.clone(),
            })
            .from_err()
            .and_then(move |db_response| {
                let merchant = db_response?;
                Ok(merchant)
            })
            .and_then(|merchant| {
                let balance = merchant.balance;
                let transfer_fee = TRANSFER_FEE;
                let knockturn_fee = (balance as f64 * KNOCKTURN_SHARE) as i64;
                let mut total = 0;

                if balance > transfer_fee + knockturn_fee {
                    total = balance - transfer_fee - knockturn_fee;
                }

                let mut template = WithdrawTemplate {
                    error: None,
                    balance: Money::from_grin(balance),
                    transfer_fee: Money::from_grin(transfer_fee),
                    knockturn_fee: Money::from_grin(knockturn_fee),
                    total: Money::from_grin(total),
                    url: "http://localhost:6000/withdraw/confirm",
                };

                if balance < MINIMAL_WITHDRAW {
                    template.error = Some(format!(
                        "You balance is too small. Minimal withdraw amount is {}",
                        Money::from_grin(MINIMAL_WITHDRAW)
                    ));
                }

                template.into_response()
            }),
    )
}

#[derive(Debug, Deserialize)]
pub struct CreatePayoutForm {
    pub amount: i64,
}

pub fn create_payout(
    (req, form): (HttpRequest<AppState>, Form<CreatePayoutForm>),
) -> FutureResponse<HttpResponse, Error> {
    let merchant_id = match req.identity() {
        Some(v) => v,
        None => return ok(HttpResponse::Found().header("location", "/login").finish()).responder(),
    };
    req.state()
        .fsm
        .send(CreatePayout {
            amount: form.amount,
            merchant_id: merchant_id,
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

#[derive(Template)]
#[template(path = "payout.html")]
struct PayoutTemplate {
    tx_id: String,
    merchant_id: String,
    status: String,
    amount: Money,
    transfer_fee: Money,
    knockturn_fee: Money,
    reminder: Money,
    download_slate: bool,
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
                tx_id: transaction.id.to_string(),
                merchant_id: transaction.merchant_id.clone(),
                status: transaction.status.to_string(),
                amount: transaction.amount.into(),
                transfer_fee: transfer_fee.into(),
                knockturn_fee: knockturn_fee.into(),
                reminder: (transaction.grin_amount - knockturn_fee - transfer_fee).into(),
                download_slate: transaction.status == TransactionStatus::New,
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
                fsm.send(InitializePayout {
                    new_payout,
                    wallet_tx,
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
    (tx_id, req, state): (Path<Uuid>, HttpRequest<AppState>, State<AppState>),
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
                        let slate = wallet.finalize(&slate);
                        slate.and_then(move |slate| {
                            fsm.send(FinalizePayout { initialized_payout })
                                .from_err()
                                .and_then(|db_response| {
                                    db_response?;
                                    Ok(())
                                })
                                .and_then(|_| ok(slate))
                        })
                    }
                })
                .and_then(|slate| Ok(HttpResponse::Ok().json(slate)))
        })
        .responder()
}

pub fn withdraw_confirmation(
    (req, state): (HttpRequest<AppState>, State<AppState>),
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
        .and_then(|_| Ok(HttpResponse::Ok().body("hello")))
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
