use crate::blocking;
use crate::db::{
    self, CreateTransaction, DbExecutor, GetMerchant, GetPayment, MarkAsReported, ReportAttempt,
    UpdateTransactionStatus,
};
use crate::errors::Error;
use crate::models::Merchant;
use crate::models::PENDING_PAYOUT_TTL_SECONDS;
use crate::models::{Confirmation, Money, Transaction, TransactionStatus, TransactionType};
use crate::wallet::TxLogEntry;
use crate::wallet::Wallet;
use actix::{Actor, Addr, Context, Handler, Message, ResponseFuture};
use actix_web::client;
use chrono::{Duration, Utc};
use derive_deref::Deref;
use diesel::pg::PgConnection;
use diesel::r2d2::{ConnectionManager, Pool};
use diesel::{self, prelude::*};
use futures::future::{Either, Future};
use log::{debug, error, warn};
use serde::{Deserialize, Serialize};
use std::fmt::Write;
use uuid::Uuid;

pub const MINIMAL_WITHDRAW: i64 = 1_000_000_000;
pub const KNOCKTURN_SHARE: f64 = 0.01;
pub const TRANSFER_FEE: i64 = 8_000_000;

pub struct Fsm {
    pub db: Addr<DbExecutor>,
    pub wallet: Wallet,
    pub pool: Pool<ConnectionManager<PgConnection>>,
}

impl Actor for Fsm {
    type Context = Context<Self>;
}

/*
 * These are messages to control Payments State Machine
 *
 */

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
pub struct UnreportedPayment(Transaction);

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

impl Message for ReportPayment<UnreportedPayment> {
    type Result = Result<(), Error>;
}

#[derive(Debug, Deserialize)]
pub struct GetNewPayment {
    pub transaction_id: Uuid,
}

impl Message for GetNewPayment {
    type Result = Result<NewPayment, Error>;
}

#[derive(Debug, Deserialize)]
pub struct GetPendingPayments;

impl Message for GetPendingPayments {
    type Result = Result<Vec<PendingPayment>, Error>;
}

#[derive(Debug, Deserialize)]
pub struct GetConfirmedPayments;

impl Message for GetConfirmedPayments {
    type Result = Result<Vec<ConfirmedPayment>, Error>;
}

#[derive(Debug, Deserialize)]
pub struct GetUnreportedPayments;

impl Message for GetUnreportedPayments {
    type Result = Result<Vec<UnreportedPayment>, Error>;
}

/*
 * These are messages to control Payout State Machine
 */

#[derive(Debug, Deserialize, Clone, Deref)]
pub struct NewPayout(Transaction);

#[derive(Debug, Serialize, Deserialize, Clone, Deref)]
pub struct InitializedPayout(Transaction);

#[derive(Debug, Serialize, Deserialize, Clone, Deref)]
pub struct PendingPayout(Transaction);

#[derive(Debug, Serialize, Deserialize, Clone, Deref)]
pub struct InChainPayout(Transaction);

#[derive(Debug, Serialize, Deserialize, Clone, Deref)]
pub struct ConfirmedPayout(Transaction);

#[derive(Debug, Serialize, Deserialize, Clone, Deref)]
pub struct RejectedPayout(Transaction);

#[derive(Debug, Deserialize)]
pub struct CreatePayout {
    pub merchant_id: String,
    pub amount: i64,
    pub confirmations: i64,
}

impl Message for CreatePayout {
    type Result = Result<NewPayout, Error>;
}

#[derive(Debug, Deserialize)]
pub struct InitializePayout {
    pub new_payout: NewPayout,
    pub wallet_tx: TxLogEntry,
    pub commit: Vec<u8>,
}

impl Message for InitializePayout {
    type Result = Result<InitializedPayout, Error>;
}

#[derive(Debug, Deserialize)]
pub struct FinalizePayout {
    pub initialized_payout: InitializedPayout,
}

impl Message for FinalizePayout {
    type Result = Result<PendingPayout, Error>;
}

#[derive(Debug, Deserialize)]
pub struct ConfirmPayout {
    pub payout: InChainPayout,
}

impl Message for ConfirmPayout {
    type Result = Result<ConfirmedPayout, Error>;
}

#[derive(Debug, Deserialize)]
pub struct RejectPayout<T> {
    pub payout: T,
}

impl Message for RejectPayout<NewPayout> {
    type Result = Result<RejectedPayout, Error>;
}

impl Message for RejectPayout<InitializedPayout> {
    type Result = Result<RejectedPayout, Error>;
}

impl Message for RejectPayout<PendingPayout> {
    type Result = Result<RejectedPayout, Error>;
}

#[derive(Debug, Deserialize)]
pub struct GetPayout {
    pub merchant_id: String,
    pub transaction_id: Uuid,
}

impl Message for GetPayout {
    type Result = Result<Transaction, Error>;
}

#[derive(Debug, Deserialize)]
pub struct GetNewPayout {
    pub transaction_id: Uuid,
    pub merchant_id: String,
}

impl Message for GetNewPayout {
    type Result = Result<NewPayout, Error>;
}

#[derive(Debug, Deserialize)]
pub struct GetInitializedPayout {
    pub transaction_id: Uuid,
    //We don't require merchant_id here because this message is used in handler
    //which accept signed slate from wallet. i.e. he won't be logged in and we wouldn't
    //know merchant_id
}

impl Message for GetInitializedPayout {
    type Result = Result<InitializedPayout, Error>;
}

#[derive(Debug, Deserialize)]
pub struct GetPendingPayouts;

impl Message for GetPendingPayouts {
    type Result = Result<Vec<PendingPayout>, Error>;
}

#[derive(Debug, Deserialize)]
pub struct GetExpiredNewPayouts;

impl Message for GetExpiredNewPayouts {
    type Result = Result<Vec<NewPayout>, Error>;
}

#[derive(Debug, Deserialize)]
pub struct GetExpiredInitializedPayouts;

impl Message for GetExpiredInitializedPayouts {
    type Result = Result<Vec<InitializedPayout>, Error>;
}

impl Handler<CreatePayment> for Fsm {
    type Result = ResponseFuture<NewPayment, Error>;

    fn handle(&mut self, msg: CreatePayment, _: &mut Self::Context) -> Self::Result {
        let create_transaction = CreateTransaction {
            merchant_id: msg.merchant_id,
            external_id: msg.external_id,
            amount: msg.amount,
            confirmations: msg.confirmations,
            email: msg.email.clone(),
            message: msg.message.clone(),
            transaction_type: TransactionType::Payment,
            redirect_url: msg.redirect_url,
        };

        let res = self
            .db
            .send(create_transaction)
            .from_err()
            .and_then(move |db_response| {
                let transaction = db_response?;
                Ok(NewPayment(transaction))
            });
        Box::new(res)
    }
}

impl Handler<GetNewPayment> for Fsm {
    type Result = ResponseFuture<NewPayment, Error>;

    fn handle(&mut self, msg: GetNewPayment, _: &mut Self::Context) -> Self::Result {
        let res = self
            .db
            .send(GetPayment {
                transaction_id: msg.transaction_id,
            })
            .from_err()
            .and_then(move |db_response| {
                let transaction = db_response?;
                if transaction.status != TransactionStatus::New {
                    return Err(Error::WrongTransactionStatus(s!(transaction.status)));
                }
                Ok(NewPayment(transaction))
            });
        Box::new(res)
    }
}

pub fn to_hex(bytes: Vec<u8>) -> String {
    let mut s = String::new();
    for byte in bytes {
        write!(&mut s, "{:02x}", byte).expect("Unable to write");
    }
    s
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

        let res = blocking::run(move || {
            use crate::schema::transactions::dsl::*;
            let conn: &PgConnection = &pool.get().unwrap();

            let transaction = diesel::update(transactions.filter(id.eq(transaction_id.clone())))
                .set((
                    wallet_tx_id.eq(msg.wallet_tx.id as i64),
                    wallet_tx_slate_id.eq(msg.wallet_tx.tx_slate_id.unwrap()),
                    slate_messages.eq(messages),
                    real_transfer_fee.eq(msg.wallet_tx.fee.map(|fee| fee as i64)),
                    status.eq(TransactionStatus::Pending),
                    commit.eq(to_hex(msg.commit)),
                ))
                .get_result(conn)
                .map_err::<Error, _>(|e| e.into())?;
            Ok(PendingPayment(transaction))
        })
        .from_err();

        Box::new(res)
    }
}

impl Handler<GetPendingPayments> for Fsm {
    type Result = ResponseFuture<Vec<PendingPayment>, Error>;

    fn handle(&mut self, _: GetPendingPayments, _: &mut Self::Context) -> Self::Result {
        Box::new(
            self.db
                .send(db::GetPaymentsByStatus(TransactionStatus::Pending))
                .from_err()
                .and_then(|db_response| {
                    let data = db_response?;
                    Ok(data.into_iter().map(PendingPayment).collect())
                }),
        )
    }
}

impl Handler<ConfirmPayment> for Fsm {
    type Result = ResponseFuture<ConfirmedPayment, Error>;

    fn handle(&mut self, msg: ConfirmPayment, _: &mut Self::Context) -> Self::Result {
        let tx_msg = db::ConfirmTransaction {
            transaction: msg.payment.0,
            confirmed_at: Some(Utc::now().naive_utc()),
        };
        Box::new(self.db.send(tx_msg).from_err().and_then(|res| {
            let tx = res?;
            Ok(ConfirmedPayment(tx))
        }))
    }
}

impl Handler<GetConfirmedPayments> for Fsm {
    type Result = ResponseFuture<Vec<ConfirmedPayment>, Error>;

    fn handle(&mut self, _: GetConfirmedPayments, _: &mut Self::Context) -> Self::Result {
        Box::new(
            self.db
                .send(db::GetPaymentsByStatus(TransactionStatus::Confirmed))
                .from_err()
                .and_then(|db_response| {
                    let data = db_response?;
                    Ok(data.into_iter().map(ConfirmedPayment).collect())
                }),
        )
    }
}

fn run_callback(
    callback_url: &str,
    token: &String,
    transaction: Transaction,
) -> impl Future<Item = (), Error = Error> {
    client::post(callback_url)
        .json(Confirmation {
            id: transaction.id,
            external_id: transaction.external_id,
            merchant_id: transaction.merchant_id,
            grin_amount: transaction.grin_amount,
            amount: transaction.amount,
            status: transaction.status,
            confirmations: transaction.confirmations,
            token: token.to_string(),
        })
        .unwrap()
        .send()
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

impl Handler<RejectPayment<NewPayment>> for Fsm {
    type Result = ResponseFuture<RejectedPayment, Error>;

    fn handle(&mut self, msg: RejectPayment<NewPayment>, _: &mut Self::Context) -> Self::Result {
        Box::new(reject_transaction(&self.db, &msg.payment.id).map(RejectedPayment))
    }
}

impl Handler<RejectPayment<PendingPayment>> for Fsm {
    type Result = ResponseFuture<RejectedPayment, Error>;

    fn handle(
        &mut self,
        msg: RejectPayment<PendingPayment>,
        _: &mut Self::Context,
    ) -> Self::Result {
        Box::new(reject_transaction(&self.db, &msg.payment.id).map(RejectedPayment))
    }
}

fn reject_transaction(
    db: &Addr<DbExecutor>,
    id: &Uuid,
) -> impl Future<Item = Transaction, Error = Error> {
    db.send(UpdateTransactionStatus {
        id: id.clone(),
        status: TransactionStatus::Rejected,
    })
    .from_err()
    .and_then(|db_response| {
        let tx = db_response?;
        Ok(tx)
    })
}

impl Handler<ReportPayment<ConfirmedPayment>> for Fsm {
    type Result = ResponseFuture<(), Error>;

    fn handle(
        &mut self,
        msg: ReportPayment<ConfirmedPayment>,
        _: &mut Self::Context,
    ) -> Self::Result {
        Box::new(report_transaction(self.db.clone(), msg.payment.0))
    }
}

impl Handler<ReportPayment<RejectedPayment>> for Fsm {
    type Result = ResponseFuture<(), Error>;

    fn handle(
        &mut self,
        msg: ReportPayment<RejectedPayment>,
        _: &mut Self::Context,
    ) -> Self::Result {
        Box::new(report_transaction(self.db.clone(), msg.payment.0))
    }
}

impl Handler<ReportPayment<UnreportedPayment>> for Fsm {
    type Result = ResponseFuture<(), Error>;

    fn handle(
        &mut self,
        msg: ReportPayment<UnreportedPayment>,
        _: &mut Self::Context,
    ) -> Self::Result {
        Box::new(report_transaction(self.db.clone(), msg.payment.0))
    }
}

fn report_transaction(
    db: Addr<DbExecutor>,
    transaction: Transaction,
) -> impl Future<Item = (), Error = Error> {
    debug!("Try to report transaction {}", transaction.id);
    db.send(GetMerchant {
        id: transaction.merchant_id.clone(),
    })
    .from_err()
    .and_then(|res| {
        let merchant = res?;
        Ok(merchant)
    })
    .and_then(move |merchant| {
        if let Some(callback_url) = merchant.callback_url.clone() {
            debug!("Run callback for merchant {}", merchant.email);
            let res = run_callback(&callback_url, &merchant.token, transaction.clone())
                .or_else({
                    let db = db.clone();
                    let transaction = transaction.clone();
                    move |callback_err| {
                        // try call ReportAttempt but ignore errors and return
                        // error from callback
                        let next_attempt = Utc::now().naive_utc()
                            + Duration::seconds(
                                10 * (transaction.report_attempts + 1).pow(2) as i64,
                            );
                        db.send(ReportAttempt {
                            transaction_id: transaction.id,
                            next_attempt: Some(next_attempt),
                        })
                        .map_err(|e| Error::General(s!(e)))
                        .and_then(|db_response| {
                            db_response?;
                            Ok(())
                        })
                        .or_else(|e| {
                            error!("Get error in ReportAttempt {}", e);
                            Ok(())
                        })
                        .and_then(|_| Err(callback_err))
                    }
                })
                .and_then({
                    let db = db.clone();
                    let transaction_id = transaction.id.clone();
                    move |_| {
                        db.send(MarkAsReported { transaction_id })
                            .from_err()
                            .and_then(|db_response| {
                                db_response?;
                                Ok(())
                            })
                    }
                });
            Either::A(res)
        } else {
            Either::B(
                db.send(MarkAsReported {
                    transaction_id: transaction.id,
                })
                .from_err()
                .and_then(|db_response| {
                    db_response?;
                    Ok(())
                }),
            )
        }
    })
}

impl Handler<GetUnreportedPayments> for Fsm {
    type Result = ResponseFuture<Vec<UnreportedPayment>, Error>;

    fn handle(&mut self, _: GetUnreportedPayments, _: &mut Self::Context) -> Self::Result {
        Box::new(
            self.db
                .send(db::GetUnreportedTransactions)
                .from_err()
                .and_then(|db_response| {
                    let data = db_response?;
                    Ok(data.into_iter().map(UnreportedPayment).collect())
                }),
        )
    }
}

impl Handler<CreatePayout> for Fsm {
    type Result = ResponseFuture<NewPayout, Error>;

    fn handle(&mut self, msg: CreatePayout, _: &mut Self::Context) -> Self::Result {
        let res = blocking::run({
            let pool = self.pool.clone();
            let merchant_id = msg.merchant_id.clone();
            move || {
                use crate::schema::merchants::dsl::*;
                let conn: &PgConnection = &pool.get().unwrap();
                let tx = conn.transaction(|| {
                    let merchant: Merchant =
                        merchants.find(merchant_id.clone()).get_result(conn)?;
                    if merchant.balance < msg.amount {
                        return Err(Error::NotEnoughFunds);
                    }

                    diesel::update(merchants.filter(id.eq(merchant_id.clone())))
                        .set(balance.eq(balance - msg.amount))
                        .get_result::<Merchant>(conn)
                        .map_err::<Error, _>(|e| e.into())?;

                    //check if fees are not too high
                    msg.amount.reminder()?;

                    let amount = Money::from_grin(msg.amount);
                    let new_id = uuid::Uuid::new_v4();
                    let new_transaction = Transaction {
                        id: new_id,
                        external_id: new_id.to_string(),
                        merchant_id: merchant_id.clone(),
                        email: Some(merchant.email.clone()),
                        amount: amount,
                        grin_amount: msg.amount,
                        status: TransactionStatus::New,
                        confirmations: msg.confirmations,
                        created_at: Utc::now().naive_utc(),
                        updated_at: Utc::now().naive_utc(),
                        report_attempts: 0,
                        next_report_attempt: None,
                        reported: false,
                        wallet_tx_id: None,
                        wallet_tx_slate_id: None,
                        message: format!(
                            "Withdrawal of {} for merchant {}",
                            amount.clone(),
                            merchant_id.clone()
                        ),
                        slate_messages: None,
                        transfer_fee: Some(msg.amount.transfer_fee()),
                        knockturn_fee: Some(msg.amount.knockturn_fee()),
                        real_transfer_fee: None,
                        transaction_type: TransactionType::Payout,
                        height: None,
                        commit: None,
                        redirect_url: None,
                    };

                    use crate::schema::transactions;
                    let tx = diesel::insert_into(transactions::table)
                        .values(&new_transaction)
                        .get_result::<Transaction>(conn)?;
                    Ok(tx)
                })?;

                Ok(NewPayout(tx))
            }
        })
        .from_err();

        Box::new(res)
    }
}

impl Handler<GetPayout> for Fsm {
    type Result = ResponseFuture<Transaction, Error>;

    fn handle(&mut self, msg: GetPayout, _: &mut Self::Context) -> Self::Result {
        let res = blocking::run({
            let pool = self.pool.clone();
            move || {
                use crate::schema::transactions::dsl::*;
                let conn: &PgConnection = &pool.get().unwrap();
                let tx = transactions
                    .filter(id.eq(msg.transaction_id.clone()))
                    .filter(merchant_id.eq(msg.merchant_id.clone()))
                    .filter(transaction_type.eq(TransactionType::Payout))
                    .first(conn)
                    .map_err(|e| e.into());
                tx
            }
        })
        .from_err();

        Box::new(res)
    }
}

impl Handler<InitializePayout> for Fsm {
    type Result = ResponseFuture<InitializedPayout, Error>;

    fn handle(&mut self, msg: InitializePayout, _: &mut Self::Context) -> Self::Result {
        let transaction_id = msg.new_payout.id.clone();
        let wallet_tx = msg.wallet_tx.clone();
        let messages: Option<Vec<String>> = wallet_tx.messages.map(|pm| {
            pm.messages
                .into_iter()
                .map(|pmd| pmd.message)
                .filter_map(|x| x)
                .collect()
        });

        let pool = self.pool.clone();

        let res = blocking::run(move || {
            use crate::schema::transactions::dsl::*;
            let conn: &PgConnection = &pool.get().unwrap();

            let transaction = diesel::update(transactions.filter(id.eq(transaction_id.clone())))
                .set((
                    wallet_tx_id.eq(msg.wallet_tx.id as i64),
                    wallet_tx_slate_id.eq(msg.wallet_tx.tx_slate_id.unwrap()),
                    slate_messages.eq(messages),
                    real_transfer_fee.eq(msg.wallet_tx.fee.map(|fee| fee as i64)),
                    status.eq(TransactionStatus::Initialized),
                    commit.eq(to_hex(msg.commit)),
                ))
                .get_result(conn)
                .map_err::<Error, _>(|e| e.into())?;
            Ok(InitializedPayout(transaction))
        })
        .from_err();

        Box::new(res)
    }
}

impl Handler<GetNewPayout> for Fsm {
    type Result = ResponseFuture<NewPayout, Error>;

    fn handle(&mut self, msg: GetNewPayout, _: &mut Self::Context) -> Self::Result {
        let res = blocking::run({
            let pool = self.pool.clone();
            move || {
                use crate::schema::transactions::dsl::*;
                let conn: &PgConnection = &pool.get().unwrap();
                transactions
                    .filter(id.eq(msg.transaction_id))
                    .filter(merchant_id.eq(msg.merchant_id))
                    .filter(status.eq(TransactionStatus::New))
                    .filter(transaction_type.eq(TransactionType::Payout))
                    .first(conn)
                    .map_err(|e| e.into())
                    .map(NewPayout)
            }
        })
        .from_err();
        Box::new(res)
    }
}

impl Handler<GetInitializedPayout> for Fsm {
    type Result = ResponseFuture<InitializedPayout, Error>;

    fn handle(&mut self, msg: GetInitializedPayout, _: &mut Self::Context) -> Self::Result {
        let res = blocking::run({
            let pool = self.pool.clone();
            move || {
                use crate::schema::transactions::dsl::*;
                let conn: &PgConnection = &pool.get().unwrap();
                transactions
                    .filter(id.eq(msg.transaction_id))
                    .filter(status.eq(TransactionStatus::Initialized))
                    .filter(transaction_type.eq(TransactionType::Payout))
                    .first(conn)
                    .map_err(|e| e.into())
                    .map(InitializedPayout)
            }
        })
        .from_err();
        Box::new(res)
    }
}

impl Handler<FinalizePayout> for Fsm {
    type Result = ResponseFuture<PendingPayout, Error>;

    fn handle(&mut self, msg: FinalizePayout, _: &mut Self::Context) -> Self::Result {
        Box::new(
            self.db
                .send(UpdateTransactionStatus {
                    id: msg.initialized_payout.id.clone(),
                    status: TransactionStatus::Pending,
                })
                .from_err()
                .and_then(|db_response| {
                    let tx = db_response?;
                    Ok(PendingPayout(tx))
                }),
        )
    }
}

impl Handler<ConfirmPayout> for Fsm {
    type Result = ResponseFuture<ConfirmedPayout, Error>;

    fn handle(&mut self, msg: ConfirmPayout, _: &mut Self::Context) -> Self::Result {
        let tx_msg = db::ConfirmTransaction {
            transaction: msg.payout.0,
            confirmed_at: Some(Utc::now().naive_utc()),
        };
        Box::new(self.db.send(tx_msg).from_err().and_then(|res| {
            let tx = res?;
            Ok(ConfirmedPayout(tx))
        }))
    }
}

impl Handler<GetPendingPayouts> for Fsm {
    type Result = ResponseFuture<Vec<PendingPayout>, Error>;

    fn handle(&mut self, _: GetPendingPayouts, _: &mut Self::Context) -> Self::Result {
        Box::new(
            self.db
                .send(db::GetPayoutsByStatus(TransactionStatus::Pending))
                .from_err()
                .and_then(|db_response| {
                    let data = db_response?;
                    Ok(data.into_iter().map(PendingPayout).collect())
                }),
        )
    }
}

impl Handler<GetExpiredNewPayouts> for Fsm {
    type Result = ResponseFuture<Vec<NewPayout>, Error>;

    fn handle(&mut self, _: GetExpiredNewPayouts, _: &mut Self::Context) -> Self::Result {
        let res = blocking::run({
            let pool = self.pool.clone();
            move || {
                use crate::schema::transactions::dsl::*;
                let conn: &PgConnection = &pool.get().unwrap();
                transactions
                    .filter(status.eq(TransactionStatus::New))
                    .filter(transaction_type.eq(TransactionType::Payout))
                    .filter(
                        created_at
                            .lt(Utc::now().naive_utc()
                                - Duration::seconds(PENDING_PAYOUT_TTL_SECONDS)),
                    )
                    .load::<Transaction>(conn)
                    .map_err(|e| e.into())
                    .map(|txs| txs.into_iter().map(NewPayout).collect())
            }
        })
        .from_err();
        Box::new(res)
    }
}

impl Handler<GetExpiredInitializedPayouts> for Fsm {
    type Result = ResponseFuture<Vec<InitializedPayout>, Error>;

    fn handle(&mut self, _: GetExpiredInitializedPayouts, _: &mut Self::Context) -> Self::Result {
        let res = blocking::run({
            let pool = self.pool.clone();
            move || {
                use crate::schema::transactions::dsl::*;
                let conn: &PgConnection = &pool.get().unwrap();
                transactions
                    .filter(status.eq(TransactionStatus::Initialized))
                    .filter(transaction_type.eq(TransactionType::Payout))
                    .filter(
                        created_at
                            .lt(Utc::now().naive_utc()
                                - Duration::seconds(PENDING_PAYOUT_TTL_SECONDS)),
                    )
                    .load::<Transaction>(conn)
                    .map_err(|e| e.into())
                    .map(|txs| txs.into_iter().map(InitializedPayout).collect())
            }
        })
        .from_err();
        Box::new(res)
    }
}

impl Handler<RejectPayout<NewPayout>> for Fsm {
    type Result = ResponseFuture<RejectedPayout, Error>;

    fn handle(&mut self, msg: RejectPayout<NewPayout>, _: &mut Self::Context) -> Self::Result {
        let res = blocking::run({
            let pool = self.pool.clone();
            move || {
                use crate::schema::merchants;
                use crate::schema::transactions;
                let conn: &PgConnection = &pool.get().unwrap();
                let tx: Transaction = conn.transaction(|| -> Result<Transaction, Error> {
                    warn!("Reject payout {:?}", msg.payout);
                    diesel::update(
                        merchants::table
                            .filter(merchants::columns::id.eq(msg.payout.merchant_id.clone())),
                    )
                    .set(
                        merchants::columns::balance
                            .eq(merchants::columns::balance + msg.payout.grin_amount),
                    )
                    .get_result::<Merchant>(conn)?;

                    let tx = diesel::update(
                        transactions::table
                            .filter(transactions::columns::id.eq(msg.payout.id.clone())),
                    )
                    .set((
                        transactions::columns::status.eq(TransactionStatus::Rejected),
                        transactions::columns::updated_at.eq(Utc::now().naive_utc()),
                    ))
                    .get_result(conn)?;
                    Ok(tx)
                })?;

                Ok(RejectedPayout(tx))
            }
        })
        .from_err();
        Box::new(res)
    }
}

impl Handler<RejectPayout<InitializedPayout>> for Fsm {
    type Result = ResponseFuture<RejectedPayout, Error>;

    fn handle(
        &mut self,
        msg: RejectPayout<InitializedPayout>,
        _: &mut Self::Context,
    ) -> Self::Result {
        let res = self
            .wallet
            .cancel_tx(&msg.payout.wallet_tx_slate_id.clone().unwrap())
            .and_then({
                let pool = self.pool.clone();
                move |_| {
                    blocking::run({
                        move || {
                            use crate::schema::merchants;
                            use crate::schema::transactions;
                            let conn: &PgConnection = &pool.get().unwrap();
                            let tx: Transaction =
                                conn.transaction(|| -> Result<Transaction, Error> {
                                    warn!("Reject payout {:?}", msg.payout);
                                    diesel::update(merchants::table.filter(
                                        merchants::columns::id.eq(msg.payout.merchant_id.clone()),
                                    ))
                                    .set(
                                        merchants::columns::balance
                                            .eq(merchants::columns::balance
                                                + msg.payout.grin_amount),
                                    )
                                    .get_result::<Merchant>(conn)?;

                                    let tx = diesel::update(transactions::table.filter(
                                        transactions::columns::id.eq(msg.payout.id.clone()),
                                    ))
                                    .set((
                                        transactions::columns::status
                                            .eq(TransactionStatus::Rejected),
                                        transactions::columns::updated_at
                                            .eq(Utc::now().naive_utc()),
                                    ))
                                    .get_result(conn)?;
                                    Ok(tx)
                                })?;
                            Ok(RejectedPayout(tx))
                        }
                    })
                    .from_err()
                }
            });

        Box::new(res)
    }
}

impl Handler<RejectPayout<PendingPayout>> for Fsm {
    type Result = ResponseFuture<RejectedPayout, Error>;

    fn handle(&mut self, msg: RejectPayout<PendingPayout>, _: &mut Self::Context) -> Self::Result {
        let res = self
            .wallet
            .cancel_tx(&msg.payout.wallet_tx_slate_id.clone().unwrap())
            .and_then({
                let pool = self.pool.clone();
                move |_| {
                    blocking::run({
                        move || {
                            use crate::schema::merchants;
                            use crate::schema::transactions;
                            let conn: &PgConnection = &pool.get().unwrap();
                            let tx: Transaction =
                                conn.transaction(|| -> Result<Transaction, Error> {
                                    warn!("Reject payout {:?}", msg.payout);
                                    diesel::update(merchants::table.filter(
                                        merchants::columns::id.eq(msg.payout.merchant_id.clone()),
                                    ))
                                    .set(
                                        merchants::columns::balance
                                            .eq(merchants::columns::balance
                                                + msg.payout.grin_amount),
                                    )
                                    .get_result::<Merchant>(conn)?;

                                    let tx = diesel::update(transactions::table.filter(
                                        transactions::columns::id.eq(msg.payout.id.clone()),
                                    ))
                                    .set((
                                        transactions::columns::status
                                            .eq(TransactionStatus::Rejected),
                                        transactions::columns::updated_at
                                            .eq(Utc::now().naive_utc()),
                                    ))
                                    .get_result(conn)?;
                                    Ok(tx)
                                })?;
                            Ok(RejectedPayout(tx))
                        }
                    })
                    .from_err()
                }
            });

        Box::new(res)
    }
}
pub trait PayoutFees {
    fn transfer_fee(&self) -> i64;
    fn knockturn_fee(&self) -> i64;
    fn reminder(&self) -> Result<i64, Error>;
}

impl PayoutFees for i64 {
    fn transfer_fee(&self) -> i64 {
        TRANSFER_FEE
    }
    fn knockturn_fee(&self) -> i64 {
        (*self as f64 * KNOCKTURN_SHARE) as i64
    }
    fn reminder(&self) -> Result<i64, Error> {
        if *self < self.transfer_fee() + self.knockturn_fee() {
            Err(Error::General(s!("fees are higher than amount")))
        } else {
            Ok(*self - self.transfer_fee() - self.knockturn_fee())
        }
    }
}

impl PayoutFees for Transaction {
    fn transfer_fee(&self) -> i64 {
        TRANSFER_FEE
    }
    fn knockturn_fee(&self) -> i64 {
        self.grin_amount.knockturn_fee()
    }
    fn reminder(&self) -> Result<i64, Error> {
        self.grin_amount.reminder()
    }
}
