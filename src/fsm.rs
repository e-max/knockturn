use crate::db::{
    create_transaction, update_transaction_status, CreateTransaction, DbExecutor, GetMerchant,
    ReportAttempt,
};
use crate::errors::Error;
use crate::models::{Confirmation, Money, Transaction, TransactionStatus, TransactionType};
use crate::ser;
use crate::wallet::TxLogEntry;
use crate::wallet::Wallet;
use crate::Pool;
use actix::{Actor, Addr, Context, Handler, Message, ResponseFuture};
use actix_web::client::Client;
use actix_web::web::block;
use chrono::{Duration, Utc};
use derive_deref::Deref;
use diesel::pg::PgConnection;
use diesel::{self, prelude::*};
use std::future::Future;
use futures::future::{ok, Either};
use log::{debug, error};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

pub const MINIMAL_WITHDRAW: i64 = 1_000_000_000;
pub const KNOCKTURN_SHARE: f64 = 0.01;
pub const TRANSFER_FEE: i64 = 8_000_000;
const MAX_REPORT_ATTEMPTS: i32 = 10; //Number or attemps we try to run merchant's callback

pub struct Fsm {
    pub db: Addr<DbExecutor>,
    pub wallet: Wallet,
    pub pool: Pool,
}

impl Actor for Fsm {
    type Context = Context<Self>;
}

/*
 * These are messages to control Payments State Machine
 *
 */

pub trait Payment
where
    Self: std::marker::Sized,
    Self: 'static,
{
    const STATUS: TransactionStatus;
    fn new(tx: Transaction) -> Self;
    fn get(tx_id: Uuid, pool: Pool) -> Box<dyn Future<Output = Result<Self, Error>>> {
        Box::new(get_payment(tx_id, Self::STATUS, pool).map(Self::new))
    }
    fn list(pool: Pool) -> Box<dyn Future<Output = Result<Vec<Self>, Error>>> {
        Box::new(
            get_payments(Self::STATUS, pool).map(|list| list.into_iter().map(Self::new).collect()),
        )
    }
}

#[derive(Debug, Serialize, Deserialize, Clone, Deref)]
pub struct NewPayment(Transaction);

#[derive(Debug, Deserialize, Clone, Deref)]
pub struct PendingPayment(Transaction);

#[derive(Debug, Deserialize, Clone, Deref)]
pub struct InChainPayment(Transaction);

#[derive(Debug, Deserialize, Clone, Deref, Serialize)]
pub struct ConfirmedPayment(Transaction);

#[derive(Debug, Deserialize, Clone, Deref)]
pub struct RejectedPayment(Transaction);

#[derive(Debug, Deserialize, Clone, Deref)]
pub struct RefundPayment(Transaction);

#[derive(Debug, Deserialize, Clone, Deref)]
pub struct ManuallyRefundedPayment(Transaction);

#[derive(Debug, Deserialize)]
pub struct CreatePayment {
    pub merchant_id: String,
    pub external_id: String,
    pub amount: Money,
    pub confirmations: i64,
    pub email: Option<String>,
    pub message: String,
    pub redirect_url: Option<String>,
}

impl Message for CreatePayment {
    type Result = Result<NewPayment, Error>;
}

#[derive(Debug, Deserialize)]
pub struct MakePayment {
    pub new_payment: NewPayment,
    pub wallet_tx: TxLogEntry,
    pub commit: Vec<u8>,
}

impl Message for MakePayment {
    type Result = Result<PendingPayment, Error>;
}

#[derive(Debug, Deserialize)]
pub struct SeenInChainPayment<T> {
    pub payment: T,
    pub height: i64,
}

impl Message for SeenInChainPayment<PendingPayment> {
    type Result = Result<InChainPayment, Error>;
}

impl Message for SeenInChainPayment<RejectedPayment> {
    type Result = Result<RefundPayment, Error>;
}

#[derive(Debug, Deserialize)]
pub struct ConfirmPayment {
    pub payment: InChainPayment,
}

impl Message for ConfirmPayment {
    type Result = Result<ConfirmedPayment, Error>;
}

#[derive(Debug, Deserialize, Deref)]
pub struct RejectPayment<T> {
    pub payment: T,
}

impl Message for RejectPayment<NewPayment> {
    type Result = Result<RejectedPayment, Error>;
}

impl Message for RejectPayment<PendingPayment> {
    type Result = Result<RejectedPayment, Error>;
}

#[derive(Debug, Deserialize, Deref)]
pub struct ReportPayment<T> {
    pub payment: T,
}

impl Message for ReportPayment<ConfirmedPayment> {
    type Result = Result<(), Error>;
}

impl Message for ReportPayment<RejectedPayment> {
    type Result = Result<(), Error>;
}

#[derive(Debug, Deserialize)]
pub struct ManuallyRefundPayment {
    pub payment: RefundPayment,
    pub merchant_id: String,
}

impl Message for ManuallyRefundPayment {
    type Result = Result<ManuallyRefundedPayment, Error>;
}

impl Handler<CreatePayment> for Fsm {
    type Result = ResponseFuture<NewPayment, Error>;

    fn handle(&mut self, msg: CreatePayment, _: &mut Self::Context) -> Self::Result {
        let tx = CreateTransaction {
            merchant_id: msg.merchant_id,
            external_id: msg.external_id,
            amount: msg.amount,
            confirmations: msg.confirmations,
            email: msg.email.clone(),
            message: msg.message.clone(),
            transaction_type: TransactionType::Payment,
            redirect_url: msg.redirect_url,
        };

        let pool = self.pool.clone();

        let res = block::<_, _, Error>(move || {
            let conn: &PgConnection = &pool.get().unwrap();
            create_transaction(tx, conn).map(|transaction| NewPayment(transaction))
        })
        .from_err();
        Box::new(res)
    }
}

async fn get_payment(
    tx_id: Uuid,
    required_status: TransactionStatus,
    pool: Pool,
) -> Result<Transaction, Error> {
     block::<_, _, Error>(move || {
        use crate::schema::transactions::dsl::*;
        let conn: &PgConnection = &pool.get().unwrap();
        transactions
            .filter(id.eq(tx_id))
            .filter(transaction_type.eq(TransactionType::Payment))
            .get_result(conn)
            .map_err(|e| e.into())
            .and_then(move |tx: Transaction| {
                if tx.status != required_status {
                    return Err(Error::WrongTransactionStatus(s!(tx.status)));
                }
                Ok(tx)
            })
    }).await.map_err(|e| e.into())
}

pub async fn get_unreported_confirmed_payments(
    pool: Pool,
) -> Result<Vec<ConfirmedPayment>, Error> {
    get_unreported_payments(TransactionStatus::Confirmed, pool).await
        .map(|list| list.into_iter().map(ConfirmedPayment).collect())
}

pub async fn get_unreported_rejected_payments(
    pool: Pool,
) -> Result<Vec<RejectedPayment>, Error> {
    get_unreported_payments(TransactionStatus::Confirmed, pool).await
        .map(|list| list.into_iter().map(RejectedPayment).collect())
}

async fn get_unreported_payments(
    tx_status: TransactionStatus,
    pool: Pool,
) -> Result<Vec<Transaction>, Error> {
    block::<_, _, Error>(move || {
        use crate::schema::transactions::dsl::*;
        let conn: &PgConnection = &pool.get().unwrap();
        transactions
            .filter(reported.ne(true))
            .filter(status.eq(tx_status))
            .filter(transaction_type.eq(TransactionType::Payment))
            .filter(report_attempts.lt(MAX_REPORT_ATTEMPTS))
            .filter(
                next_report_attempt
                    .le(Utc::now().naive_utc())
                    .or(next_report_attempt.is_null()),
            )
            .load::<Transaction>(conn)
            .map_err(|e| e.into())
    })
    .await
    .map_err(|e| e.into())
}

async fn get_payments(
    tx_status: TransactionStatus,
    pool: Pool,
) -> Result<Vec<Transaction>, Error> {
    block::<_, _, Error>(move || {
        use crate::schema::transactions::dsl::*;
        let conn: &PgConnection = &pool.get().unwrap();
        transactions
            .filter(transaction_type.eq(TransactionType::Payment))
            .filter(status.eq(tx_status))
            .load::<Transaction>(conn)
            .map_err(|e| e.into())
    })
    .await.map_err(|e| e.into())
}

impl Payment for NewPayment {
    const STATUS: TransactionStatus = TransactionStatus::New;
    fn new(tx: Transaction) -> Self {
        Self(tx)
    }
}

impl Payment for PendingPayment {
    const STATUS: TransactionStatus = TransactionStatus::Pending;
    fn new(tx: Transaction) -> Self {
        Self(tx)
    }
}

impl Payment for ConfirmedPayment {
    const STATUS: TransactionStatus = TransactionStatus::Confirmed;
    fn new(tx: Transaction) -> Self {
        Self(tx)
    }
}

impl Payment for RefundPayment {
    const STATUS: TransactionStatus = TransactionStatus::Refund;
    fn new(tx: Transaction) -> Self {
        Self(tx)
    }
}

impl Handler<MakePayment> for Fsm {
    type Result = ResponseFuture<PendingPayment, Error>;

    fn handle(&mut self, msg: MakePayment, _: &mut Self::Context) -> Self::Result {
        let transaction_id = msg.new_payment.id.clone();
        let wallet_tx = msg.wallet_tx.clone();
        let messages: Option<Vec<String>> = wallet_tx.messages.map(|pm| {
            pm.messages
                .into_iter()
                .map(|pmd| pmd.message)
                .filter_map(|x| x)
                .collect()
        });

        let pool = self.pool.clone();

        let res = block::<_, _, Error>(move || {
            use crate::schema::transactions::dsl::*;
            let conn: &PgConnection = &pool.get().unwrap();

            let transaction = diesel::update(transactions.filter(id.eq(transaction_id.clone())))
                .set((
                    wallet_tx_id.eq(msg.wallet_tx.id as i64),
                    wallet_tx_slate_id.eq(msg.wallet_tx.tx_slate_id.unwrap()),
                    slate_messages.eq(messages),
                    real_transfer_fee.eq(msg.wallet_tx.fee.map(|fee| fee as i64)),
                    status.eq(TransactionStatus::Pending),
                    commit.eq(ser::to_hex(msg.commit)),
                ))
                .get_result(conn)
                .map_err::<Error, _>(|e| e.into())?;
            Ok(PendingPayment(transaction))
        })
        .from_err();

        Box::new(res)
    }
}

impl Handler<SeenInChainPayment<PendingPayment>> for Fsm {
    type Result = ResponseFuture<InChainPayment, Error>;

    fn handle(
        &mut self,
        msg: SeenInChainPayment<PendingPayment>,
        _: &mut Self::Context,
    ) -> Self::Result {
        Box::new(
            block::<_, _, Error>({
                let pool = self.pool.clone();
                move || {
                    use crate::schema::transactions::dsl::*;
                    let conn: &PgConnection = &pool.get().unwrap();
                    Ok(
                        diesel::update(transactions.filter(id.eq(msg.payment.id.clone())))
                            .set((height.eq(msg.height), status.eq(TransactionStatus::InChain)))
                            .get_result(conn)
                            .map(|tx: Transaction| InChainPayment(tx))
                            .map_err::<Error, _>(|e| e.into())?,
                    )
                }
            })
            .from_err(),
        )
    }
}

impl Handler<SeenInChainPayment<RejectedPayment>> for Fsm {
    type Result = ResponseFuture<RefundPayment, Error>;

    fn handle(
        &mut self,
        msg: SeenInChainPayment<RejectedPayment>,
        _: &mut Self::Context,
    ) -> Self::Result {
        Box::new(
            block::<_, _, Error>({
                let pool = self.pool.clone();
                move || {
                    use crate::schema::transactions::dsl::*;
                    let conn: &PgConnection = &pool.get().unwrap();
                    Ok(
                        diesel::update(transactions.filter(id.eq(msg.payment.id.clone())))
                            .set(status.eq(TransactionStatus::Refund))
                            .get_result(conn)
                            .map(|tx: Transaction| RefundPayment(tx))
                            .map_err::<Error, _>(|e| e.into())?,
                    )
                }
            })
            .from_err(),
        )
    }
}

impl Handler<ConfirmPayment> for Fsm {
    type Result = ResponseFuture<ConfirmedPayment, Error>;

    fn handle(&mut self, msg: ConfirmPayment, _: &mut Self::Context) -> Self::Result {
        Box::new(
            block::<_, _, Error>({
                let pool = self.pool.clone();
                move || {
                    use crate::schema::transactions::dsl::*;
                    let conn: &PgConnection = &pool.get().unwrap();

                    let tx = diesel::update(transactions.filter(id.eq(msg.payment.id)))
                        .set((
                            status.eq(TransactionStatus::Confirmed),
                            updated_at.eq(Utc::now().naive_utc()),
                        ))
                        .get_result(conn)?;
                    Ok(ConfirmedPayment(tx))
                }
            })
            .from_err(),
        )
    }
}

async fn run_callback(
    callback_url: &str,
    token: &str,
    transaction: &Transaction,
) -> Result<(), Error> {
    let confirmation = Confirmation {
        id: transaction.id.clone(),
        external_id: transaction.external_id.clone(),
        merchant_id: transaction.merchant_id.clone(),
        grin_amount: transaction.grin_amount,
        amount: transaction.amount.clone(),
        status: transaction.status.clone(),
        confirmations: transaction.confirmations.clone(),
        token: token.to_owned(),
    };
    let resp = Client::default()
        .post(callback_url)
        .send_json(&confirmation)
        .await
        .map_err({
            let callback_url = callback_url.to_owned();
            move |e| Error::MerchantCallbackError {
                callback_url: callback_url,
                error: s!(e),
            }
        })?;

    if resp.status().is_success() {
        Ok(())
    } else {
        Err(Error::MerchantCallbackError {
            callback_url: callback_url.to_owned(),
            error: s!("aaa"),
        })
    }
}

/*

fn run_callback<'a>(
    callback_url: &'a str,
    token: &'a str,
    transaction: &'a Transaction,
) -> impl Future<Item = ()> + 'a {
    Client::default()
        .post(callback_url)
        .send_json(&Confirmation {
            id: &transaction.id,
            external_id: &transaction.external_id,
            merchant_id: &transaction.merchant_id,
            grin_amount: transaction.grin_amount,
            amount: &transaction.amount,
            status: transaction.status,
            confirmations: transaction.confirmations,
            token: token,
        })
        .map_err({
            let callback_url = callback_url.to_owned();
            move |e| Error::MerchantCallbackError {
                callback_url: callback_url,
                error: s!(e),
            }
        })
        .and_then({
            let callback_url = callback_url.to_owned();
            |resp| {
                if resp.status().is_success() {
                    Ok(())
                } else {
                    Err(Error::MerchantCallbackError {
                        callback_url: callback_url,
                        error: s!("aaa"),
                    })
                }
            }
        })
}

*/

impl Handler<ManuallyRefundPayment> for Fsm {
    type Result = ResponseFuture<ManuallyRefundedPayment, Error>;

    fn handle(&mut self, msg: ManuallyRefundPayment, _: &mut Self::Context) -> Self::Result {
        Box::new(
            block::<_, _, Error>({
                let merch_id = msg.merchant_id.clone();
                let pool = self.pool.clone();
                let transaction_id = msg.payment.id.clone();
                move || {
                    use crate::schema::transactions::dsl::*;
                    let conn: &PgConnection = &pool.get().unwrap();

                    diesel::update(
                        transactions
                            .filter(id.eq(transaction_id))
                            .filter(merchant_id.eq(merch_id))
                            .filter(status.eq(TransactionStatus::Refund)),
                    )
                    .set((
                        status.eq(TransactionStatus::RefundedManually),
                        updated_at.eq(Utc::now().naive_utc()),
                    ))
                    .get_result::<Transaction>(conn)
                    .map_err::<Error, _>(|e| e.into())
                    .map(ManuallyRefundedPayment)
                }
            })
            .from_err(),
        )
    }
}

impl Handler<RejectPayment<NewPayment>> for Fsm {
    type Result = ResponseFuture<RejectedPayment, Error>;

    fn handle(&mut self, msg: RejectPayment<NewPayment>, _: &mut Self::Context) -> Self::Result {
        Box::new(reject_transaction(self.pool.clone(), &msg.payment.id).map(RejectedPayment))
    }
}

impl Handler<RejectPayment<PendingPayment>> for Fsm {
    type Result = ResponseFuture<RejectedPayment, Error>;

    fn handle(
        &mut self,
        msg: RejectPayment<PendingPayment>,
        _: &mut Self::Context,
    ) -> Self::Result {
        Box::new(reject_transaction(self.pool.clone(), &msg.payment.id).map(RejectedPayment))
    }
}

async fn reject_transaction(pool: Pool, id: &Uuid) -> Result<Transaction, Error> {
    let id = id.clone();
    block::<_, _, Error>(move || {
        let conn: &PgConnection = &pool.get().unwrap();
        update_transaction_status(id, TransactionStatus::Rejected, conn)
    })
    .await
    .map_err(|e| e.into())
}

impl Handler<ReportPayment<ConfirmedPayment>> for Fsm {
    type Result = ResponseFuture<(), Error>;

    fn handle(
        &mut self,
        msg: ReportPayment<ConfirmedPayment>,
        _: &mut Self::Context,
    ) -> Self::Result {
        Box::new(
            report_transaction(self.db.clone(), msg.payment.0.clone()).and_then({
                let pool = self.pool.clone();
                move |_| {
                    block::<_, _, Error>({
                        move || {
                            let conn: &PgConnection = &pool.get().unwrap();
                            use crate::schema::transactions::dsl::*;
                            diesel::update(transactions.filter(id.eq(msg.payment.id)))
                                .set(reported.eq(true))
                                .get_result::<Transaction>(conn)
                                .map_err::<Error, _>(|e| e.into())?;
                            Ok(())
                        }
                    })
                    .from_err()
                }
            }),
        )
    }
}

impl Handler<ReportPayment<RejectedPayment>> for Fsm {
    type Result = ResponseFuture<(), Error>;

    fn handle(
        &mut self,
        msg: ReportPayment<RejectedPayment>,
        _: &mut Self::Context,
    ) -> Self::Result {
        Box::new(
            report_transaction(self.db.clone(), msg.payment.0.clone()).and_then({
                let pool = self.pool.clone();
                move |_| {
                    block::<_, _, Error>({
                        move || {
                            let conn: &PgConnection = &pool.get().unwrap();
                            conn.transaction(|| {
                                use crate::schema::transactions::dsl::*;
                                diesel::update(transactions.filter(id.eq(msg.payment.id)))
                                    .set(reported.eq(true))
                                    .get_result::<Transaction>(conn)
                                    .map_err::<Error, _>(|e| e.into())?;

                                Ok(())
                            })
                        }
                    })
                    .from_err()
                }
            }),
        )
    }
}

async fn report_transaction(
    db: Addr<DbExecutor>,
    transaction: Transaction,
) -> Result<(), Error> {
    debug!("Try to report transaction {}", transaction.id);

    let merchant = db.send(GetMerchant {
        id: transaction.merchant_id.clone(),
    }).await??;


    if let Some(callback_url) = merchant.callback_url.clone() {
        debug!("Run callback for merchant {}", merchant.email);

        if let Err(callback_err) = run_callback(&callback_url, &merchant.token, &transaction).await {

            let report_attempts = transaction.report_attempts.clone();
            let transaction_id = transaction.id.clone();
            let next_attempt = Utc::now().naive_utc()
                + Duration::seconds(10 * (report_attempts + 1).pow(2) as i64);
            let dbres = db.send(ReportAttempt {
                transaction_id: transaction_id,
                next_attempt: Some(next_attempt),
            })
            .await
            .map_err(|e| Error::General(s!(e)))
            .and_then(|db_response| {
                db_response?;
                Ok(())
            });

            if let Err(e) = dbres {
                error!("Get error in ReportAttempt {}", e);
            };


            return Err(callback_err)
        }

    };

    Ok(())

}
