use crate::app::AppState;
use crate::db::get_balance;
use crate::errors::*;
use crate::extractor::{SimpleJson, User};
use crate::filters::{self, ForHuman};
use crate::fsm::MINIMAL_WITHDRAW;
use crate::fsm_payout::{
    CreatePayout, FinalizePayout, GetInitializedPayout, GetNewPayout, GetPayout, InitializePayout,
    PayoutFees,
};
use crate::handlers::check_2fa_code;
use crate::handlers::BootstrapColor;
use crate::models::{Merchant, Money, Transaction, TransactionStatus};
use crate::wallet::Slate;
use actix_identity::Identity;
use actix_web::web::{block, Data, Form, Path};
use actix_web::{HttpRequest, HttpResponse};
use diesel::pg::PgConnection;

use askama::Template;
use futures::future::{ok, Either, Future};
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

pub async fn withdraw(
    merchant: User<Merchant>,
    data: Data<AppState>,
) -> Result<HttpResponse, Error> {
    block::<_, _, Error>({
        let pool = data.pool.clone();
        move || {
            let conn: &PgConnection = &pool.get().unwrap();
            let balance = get_balance(&merchant.id, conn)?;
            let reminder = balance.reminder().unwrap_or(0);
            let mut template = WithdrawTemplate {
                error: None,
                balance: balance.into(),
                transfer_fee: balance.transfer_fee().into(),
                knockturn_fee: balance.knockturn_fee().into(),
                total: reminder.into(),
                url: "http://localhost:6000/withdraw/confirm",
            };

            if balance < MINIMAL_WITHDRAW {
                template.error = Some(format!(
                    "You balance is too small. Minimal withdraw amount is {}",
                    Money::from_grin(MINIMAL_WITHDRAW)
                ));
            }

            template.render().map_err(|e| Error::from(e))
        }
    })
    .await
    .map_err(|e| e.into())
    .and_then(|html| Ok(HttpResponse::Ok().content_type("text/html").body(html)))
}

#[derive(Debug, Deserialize)]
pub struct CreatePayoutForm {
    pub amount: i64,
    pub code: String,
}

pub async fn create_payout(
    data: Data<AppState>,
    form: Form<CreatePayoutForm>,
    identity_merchant: User<Merchant>,
) -> Result<HttpResponse, Error> {
    let merchant = identity_merchant.clone().into_inner();
    let code = form.code.clone();
    let validated = check_2fa_code(&merchant, &code)?;

    if !validated {
        withdraw(identity_merchant, data).await
    } else {
        let resp = data
            .fsm_payout
            .send(CreatePayout {
                amount: form.amount,
                merchant_id: merchant.id,
                confirmations: 10,
            })
            .await?;

        let payout = resp?;
        Ok(HttpResponse::Found()
            .header("location", format!("/payouts/{}", payout.id))
            .finish())
    }
}

#[derive(Template)]
#[template(path = "payout.html")]
struct PayoutTemplate<'a> {
    payout: &'a Transaction,
}

pub async fn get_payout(
    transaction_id: Path<Uuid>,
    state: Data<AppState>,
    id: Identity,
) -> Result<HttpResponse, Error> {
    let merchant_id = id.identity().unwrap();

    let transaction = state
        .fsm_payout
        .send(GetPayout {
            merchant_id: merchant_id,
            transaction_id: transaction_id.clone(),
        })
        .await??;
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
}

pub async fn generate_slate(
    transaction_id: Path<Uuid>,
    state: Data<AppState>,
    id: Identity,
) -> Result<HttpResponse, Error> {
    let merchant_id = match id.identity() {
        Some(v) => v,
        None => return Ok(HttpResponse::Found().header("location", "/login").finish()),
    };

    let new_payout = state
        .fsm_payout
        .send(GetNewPayout {
            merchant_id: merchant_id,
            transaction_id: transaction_id.clone(),
        })
        .await??;

    let real_payment = new_payout.grin_amount
        - new_payout.transfer_fee.unwrap()
        - new_payout.knockturn_fee.unwrap();

    let resp = state
        .wallet
        .create_slate(real_payment as u64, new_payout.message.clone())
        .await?;

    let slate = resp.into_result()?;

    let wallet_tx = state
        .wallet
        .get_tx(&slate.id.hyphenated().to_string())
        .await?;

    let commit = slate.tx.output_commitments()[0].clone();

    state
        .fsm_payout
        .send(InitializePayout {
            new_payout,
            wallet_tx,
            commit,
        })
        .await?;

    Ok(HttpResponse::Ok()
        .content_type("application/octet-stream")
        .json(resp.into_inner()))
}

pub async fn accept_slate(
    slate: SimpleJson<Slate>,
    tx_id: Path<Uuid>,
    state: Data<AppState>,
) -> Result<HttpResponse, Error> {
    let initialized_payout = state
        .fsm_payout
        .send(GetInitializedPayout {
            transaction_id: tx_id.clone(),
        })
        .await??;
    let finalized_slate = state.wallet.finalize(&slate).await?;
    state.wallet.post_tx().await?;

    state
        .fsm_payout
        .send(FinalizePayout { initialized_payout })
        .await??;
    Ok(HttpResponse::Ok().json(finalized_slate))
}

pub fn withdraw_confirmation(_req: HttpRequest) -> Result<HttpResponse, Error> {
    Ok(HttpResponse::Ok().body("hello"))
}
