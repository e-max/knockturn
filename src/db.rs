use crate::errors::*;
use crate::models::{
    Currency, Merchant, Money, Rate, Transaction, TransactionStatus, TransactionType,
    NEW_PAYMENT_TTL_SECONDS,
};
use crate::Pool;
use actix::{Actor, SyncContext};
use actix::{Handler, Message};
use bigdecimal::{BigDecimal, ToPrimitive};
use chrono::NaiveDateTime;
use chrono::{Duration, Local, Utc};
use data_encoding::BASE32;
use diesel::dsl::sum;
use diesel::pg::PgConnection;
use diesel::{self, prelude::*};
use log::info;
use rand::seq::SliceRandom;
use rand::{thread_rng, Rng};
use serde::Deserialize;
use std::collections::HashMap;
use uuid::Uuid;

pub struct DbExecutor(pub Pool);

impl Actor for DbExecutor {
    type Context = SyncContext<Self>;
}

#[derive(Debug, Deserialize, Default)]
pub struct CreateMerchant {
    pub id: String,
    pub email: String,
    pub password: String,
    pub wallet_url: Option<String>,
    pub callback_url: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct GetMerchant {
    pub id: String,
}

#[derive(Debug, Deserialize, Default)]
pub struct CreateTransaction {
    pub merchant_id: String,
    pub external_id: String,
    pub amount: Money,
    pub confirmations: i64,
    pub email: Option<String>,
    pub message: String,
    pub transaction_type: TransactionType,
    pub redirect_url: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct ConvertCurrency {
    pub amount: Money,
    pub to: String,
}

#[derive(Debug, Deserialize)]
pub struct GetPayoutsByStatus(pub TransactionStatus);

#[derive(Debug, Deserialize)]
pub struct ReportAttempt {
    pub transaction_id: Uuid,
    pub next_attempt: Option<NaiveDateTime>,
}

#[derive(Debug, Deserialize)]
pub struct Confirm2FA {
    pub merchant_id: String,
}

#[derive(Debug, Deserialize)]
pub struct Reset2FA {
    pub merchant_id: String,
}

#[derive(Debug, Deserialize)]
pub struct RejectExpiredPayments;

impl Message for GetMerchant {
    type Result = Result<Merchant, Error>;
}

impl Message for GetPayoutsByStatus {
    type Result = Result<Vec<Transaction>, Error>;
}

impl Message for ConvertCurrency {
    type Result = Result<Money, Error>;
}

impl Message for ReportAttempt {
    type Result = Result<(), Error>;
}

impl Message for Confirm2FA {
    type Result = Result<(), Error>;
}

impl Message for Reset2FA {
    type Result = Result<(), Error>;
}

impl Message for RejectExpiredPayments {
    type Result = Result<(), Error>;
}

pub fn create_merchant(m: CreateMerchant, conn: &PgConnection) -> Result<Merchant, Error> {
    use crate::schema::merchants;
    const CHARSET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZ\
    abcdefghijklmnopqrstuvwxyz\
    0123456789";
    let password =
        bcrypt::hash(&m.password, bcrypt::DEFAULT_COST).map_err(|e| Error::General(s!(e)))?;

    let mut rng = thread_rng();
    let new_token: Option<String> = (0..64)
        .map(|_| Some(*CHARSET.choose(&mut rng)? as char))
        .collect();
    let new_token_2fa = BASE32.encode(&rng.gen::<[u8; 10]>());
    let new_merchant = Merchant {
        id: m.id,
        email: m.email,
        password: password,
        wallet_url: m.wallet_url,
        created_at: Local::now().naive_local() + Duration::hours(24),
        callback_url: m.callback_url,
        token: new_token.ok_or(Error::General(s!("cannot generate rangom token")))?,
        token_2fa: Some(new_token_2fa),
        confirmed_2fa: false,
    };

    diesel::insert_into(merchants::table)
        .values(&new_merchant)
        .get_result(conn)
        .map_err(|e| e.into())
}

impl Handler<GetMerchant> for DbExecutor {
    type Result = Result<Merchant, Error>;

    fn handle(&mut self, msg: GetMerchant, _: &mut Self::Context) -> Self::Result {
        use crate::schema::merchants::dsl::*;
        let conn: &PgConnection = &self.0.get().unwrap();
        merchants
            .find(msg.id)
            .get_result(conn)
            .map_err(|e| e.into())
    }
}

pub fn get_transaction(transaction_id: Uuid, conn: &PgConnection) -> Result<Transaction, Error> {
    use crate::schema::transactions::dsl::*;
    transactions
        .find(transaction_id)
        .get_result(conn)
        .map_err(|e| e.into())
}

impl Handler<GetPayoutsByStatus> for DbExecutor {
    type Result = Result<Vec<Transaction>, Error>;

    fn handle(&mut self, msg: GetPayoutsByStatus, _: &mut Self::Context) -> Self::Result {
        use crate::schema::transactions::dsl::*;
        let conn: &PgConnection = &self.0.get().unwrap();
        transactions
            .filter(transaction_type.eq(TransactionType::Payout))
            .filter(status.eq(msg.0))
            .load::<Transaction>(conn)
            .map_err(|e| e.into())
    }
}

pub fn create_transaction(
    tx: CreateTransaction,
    conn: &PgConnection,
) -> Result<Transaction, Error> {
    use crate::schema::merchants::dsl::*;
    use crate::schema::rates::dsl::*;
    use crate::schema::transactions::dsl::*;

    if !merchants
        .find(tx.merchant_id.clone())
        .get_result::<Merchant>(conn)
        .is_ok()
    {
        return Err(Error::InvalidEntity("merchant".to_owned()));
    }

    let exch_rate = match rates
        .find(&tx.amount.currency.to_string())
        .get_result::<Rate>(conn)
        .optional()?
    {
        None => return Err(Error::UnsupportedCurrency(tx.amount.currency.to_string())),
        Some(v) => v,
    };

    let grins = tx.amount.convert_to(Currency::GRIN, exch_rate.rate);

    let new_transaction = Transaction {
        id: uuid::Uuid::new_v4(),
        external_id: tx.external_id,
        merchant_id: tx.merchant_id,
        email: tx.email,
        amount: tx.amount,
        grin_amount: grins.amount,
        status: TransactionStatus::New,
        confirmations: tx.confirmations,
        created_at: Local::now().naive_local(),
        updated_at: Local::now().naive_local(),
        report_attempts: 0,
        next_report_attempt: None,
        reported: false,
        wallet_tx_id: None,
        wallet_tx_slate_id: None,
        message: tx.message,
        slate_messages: None,
        transfer_fee: None,
        knockturn_fee: None,
        real_transfer_fee: None,
        transaction_type: tx.transaction_type,
        height: None,
        commit: None,
        redirect_url: tx.redirect_url,
    };

    diesel::insert_into(transactions)
        .values(&new_transaction)
        .get_result(conn)
        .map_err(|e| e.into())
}

pub fn update_transaction_status(
    tx_id: Uuid,
    tx_status: TransactionStatus,
    conn: &PgConnection,
) -> Result<Transaction, Error> {
    use crate::schema::transactions::dsl::*;
    diesel::update(transactions.filter(id.eq(tx_id)))
        .set((status.eq(tx_status), updated_at.eq(Utc::now().naive_utc())))
        .get_result(conn)
        .map_err(|e| e.into())
}

pub fn register_rate(rates_map: HashMap<String, f64>, conn: &PgConnection) -> Result<(), Error> {
    use crate::schema::rates::dsl::*;

    for (currency, new_rate) in rates_map {
        let new_rate = Rate {
            id: currency.to_uppercase(),
            rate: new_rate,
            updated_at: Local::now().naive_local(),
        };

        diesel::insert_into(rates)
            .values(&new_rate)
            .on_conflict(id)
            .do_update()
            .set(&new_rate)
            .get_result::<Rate>(conn)
            .map_err(|e| Error::from(e))?;
    }
    Ok(())
}

pub fn report_attempt(
    conn: &PgConnection,
    transaction_id: Uuid,
    next_attempt: Option<NaiveDateTime>,
) -> Result<(), Error> {
    use crate::schema::transactions::dsl::*;
    let next_attempt = next_attempt.unwrap_or(Utc::now().naive_utc() + Duration::seconds(10));
    diesel::update(transactions.filter(id.eq(transaction_id)))
        .set((
            report_attempts.eq(report_attempts + 1),
            next_report_attempt.eq(next_attempt),
        ))
        .get_result(conn)
        .map_err(|e| e.into())
        .map(|_: Transaction| ())
}

impl Handler<ReportAttempt> for DbExecutor {
    type Result = Result<(), Error>;

    fn handle(&mut self, msg: ReportAttempt, _: &mut Self::Context) -> Self::Result {
        report_attempt(&self.0.get().unwrap(), msg.transaction_id, msg.next_attempt)
    }
}

impl Handler<Confirm2FA> for DbExecutor {
    type Result = Result<(), Error>;

    fn handle(&mut self, msg: Confirm2FA, _: &mut Self::Context) -> Self::Result {
        info!("Confirm 2fa token for merchant {}", msg.merchant_id);
        use crate::schema::merchants::dsl::*;
        let conn: &PgConnection = &self.0.get().unwrap();
        diesel::update(merchants.filter(id.eq(msg.merchant_id)))
            .set((confirmed_2fa.eq(true),))
            .get_result(conn)
            .map_err(|e| e.into())
            .map(|_: Merchant| ())
    }
}

impl Handler<Reset2FA> for DbExecutor {
    type Result = Result<(), Error>;

    fn handle(&mut self, msg: Reset2FA, _: &mut Self::Context) -> Self::Result {
        info!("Confirm 2fa token for merchant {}", msg.merchant_id);
        use crate::schema::merchants::dsl::*;
        let conn: &PgConnection = &self.0.get().unwrap();

        let new_token_2fa = BASE32.encode(&thread_rng().gen::<[u8; 10]>());
        diesel::update(merchants.filter(id.eq(msg.merchant_id)))
            .set((confirmed_2fa.eq(false), token_2fa.eq(new_token_2fa)))
            .get_result(conn)
            .map_err(|e| e.into())
            .map(|_: Merchant| ())
    }
}

impl Handler<RejectExpiredPayments> for DbExecutor {
    type Result = Result<(), Error>;

    fn handle(&mut self, _: RejectExpiredPayments, _: &mut Self::Context) -> Self::Result {
        use crate::schema::transactions::dsl::*;
        let conn: &PgConnection = &self.0.get().unwrap();
        diesel::update(
            transactions
                .filter(status.eq(TransactionStatus::New))
                .filter(transaction_type.eq(TransactionType::Payment))
                .filter(
                    created_at
                        .lt(Utc::now().naive_utc() - Duration::seconds(NEW_PAYMENT_TTL_SECONDS)),
                ),
        )
        .set(status.eq(TransactionStatus::Rejected))
        .execute(conn)
        .map_err(|e| e.into())
        .map(|n| {
            if n > 0 {
                info!("Rejected {} expired new payments", n);
            }
            ()
        })
    }
}
pub fn get_current_height(conn: &PgConnection) -> Result<i64, Error> {
    use crate::schema::current_height::dsl::*;
    current_height
        .select(height)
        .first(conn)
        .map_err(|e| e.into())
}

//
//  SELECT (
//              SELECT coalesce(sum(grin_amount), 0)
//                  FROM transactions
//                  WHERE merchant_id = 'id' AND transaction_type = 'payment' AND ( status = 'refund' OR status = 'refunded_manually' OR  ( status = 'confirmed' and reported = true))
//          )
//          -
//          (
//              SELECT coalesce(sum(grin_amount), 0)
//                  FROM transactions
//                  WHERE merchant_id = 'id' AND transaction_type = 'payout' AND status <> 'rejected'
//          );
//
pub fn get_balance(merch_id: &str, conn: &PgConnection) -> Result<i64, Error> {
    use crate::schema::transactions::dsl::*;
    let payments = transactions
        .select(sum(grin_amount))
        .filter(merchant_id.eq(merch_id))
        .filter(
            // As a valid we consider a payments with
            // Status=Confirmed and reported to merchant (which means that user got his goods)
            // or Status = Refund (which means that we took user's money, but couldn't report to merchant)
            status
                .eq(TransactionStatus::Refund)
                .or(status.eq(TransactionStatus::RefundedManually))
                .or(status
                    .eq(TransactionStatus::Confirmed)
                    .and(reported.eq(true))),
        )
        .filter(transaction_type.eq(TransactionType::Payment))
        .first::<Option<BigDecimal>>(conn)
        .map_err::<Error, _>(|e| e.into())?
        .and_then(|b| b.to_i64())
        .unwrap_or(0);

    let payouts = transactions
        .select(sum(grin_amount))
        .filter(merchant_id.eq(merch_id))
        .filter(status.ne(TransactionStatus::Rejected))
        .filter(transaction_type.eq(TransactionType::Payout))
        .first::<Option<BigDecimal>>(conn)
        .map_err::<Error, _>(|e| e.into())?
        .and_then(|b| b.to_i64())
        .unwrap_or(0);

    Ok(payments - payouts)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::errors::Error;
    use crate::test_utils::{get_test_pool, run_migrations};
    use diesel::Connection;
    use diesel::{self, prelude::*};

    #[test]
    fn balance_test() {
        let pool = get_test_pool();

        let conn = pool.get().unwrap();
        conn.test_transaction::<(), Error, _>(|| {
            run_migrations(&conn);
            create_merchant(
                CreateMerchant {
                    id: s!("user"),
                    ..Default::default()
                },
                &conn,
            )
            .unwrap();
            assert!(get_balance("user", &conn).unwrap() == 0);
            let mut rates = HashMap::new();
            rates.insert(s!("grin"), 1.0);
            register_rate(rates, &conn).unwrap();
            create_transaction(
                CreateTransaction {
                    merchant_id: s!("user"),
                    external_id: s!("1"),
                    amount: Money::from_grin(1),
                    ..Default::default()
                },
                &conn,
            )
            .unwrap();
            assert!(get_balance("user", &conn).unwrap() == 0);

            // Test that payment doesn't appear on balance during processing
            let tx = create_transaction(
                CreateTransaction {
                    merchant_id: s!("user"),
                    external_id: s!("2"),
                    amount: Money::from_grin(1),
                    ..Default::default()
                },
                &conn,
            )
            .unwrap();
            assert!(get_balance("user", &conn).unwrap() == 0);

            update_transaction_status(tx.id, TransactionStatus::Pending, &conn).unwrap();
            assert!(get_balance("user", &conn).unwrap() == 0);
            update_transaction_status(tx.id, TransactionStatus::Rejected, &conn).unwrap();
            assert!(get_balance("user", &conn).unwrap() == 0);
            update_transaction_status(tx.id, TransactionStatus::InChain, &conn).unwrap();
            assert!(get_balance("user", &conn).unwrap() == 0);

            // test that Confirmed payment increases balance only if reported
            update_transaction_status(tx.id, TransactionStatus::Confirmed, &conn).unwrap();
            assert!(get_balance("user", &conn).unwrap() == 0);

            use crate::schema::transactions::dsl::*;
            diesel::update(transactions.filter(id.eq(tx.id)))
                .set(reported.eq(true))
                .get_result::<Transaction>(&conn)
                .unwrap();

            assert!(get_balance("user", &conn).unwrap() == 1);

            //text that Refund added to balance
            let tx2 = create_transaction(
                CreateTransaction {
                    merchant_id: s!("user"),
                    external_id: s!("3"),
                    amount: Money::from_grin(1),
                    ..Default::default()
                },
                &conn,
            )
            .unwrap();
            assert!(get_balance("user", &conn).unwrap() == 1);

            update_transaction_status(tx2.id, TransactionStatus::Refund, &conn).unwrap();
            assert!(get_balance("user", &conn).unwrap() == 2);

            // test that payouts reduces balance
            let payout = create_transaction(
                CreateTransaction {
                    merchant_id: s!("user"),
                    external_id: s!("-"),
                    transaction_type: TransactionType::Payout,
                    amount: Money::from_grin(1),
                    ..Default::default()
                },
                &conn,
            )
            .unwrap();
            assert!(get_balance("user", &conn).unwrap() == 1);

            update_transaction_status(payout.id, TransactionStatus::Confirmed, &conn).unwrap();
            assert!(get_balance("user", &conn).unwrap() == 1);

            // test that Rejected payouts ignored
            update_transaction_status(payout.id, TransactionStatus::Rejected, &conn).unwrap();
            assert!(get_balance("user", &conn).unwrap() == 2);

            Ok(())
        });
    }
}
