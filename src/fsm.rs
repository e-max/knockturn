use crate::blocking;
use crate::clients::BearerTokenAuth;
use crate::db::{
    self, CreateTransaction, DbExecutor, GetMerchant, GetTransaction, MarkAsReported,
    ReportAttempt, UpdateTransactionStatus, UpdateTransactionWithTxLog,
};
use crate::errors::Error;
use crate::models::Merchant;
use crate::models::{Money, Transaction, TransactionStatus};
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
use log::{debug, error};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

pub const MINIMAL_WITHDRAW: i64 = 1_000_000_000;
pub const KNOCKTURN_FEE: i64 = 0;
pub const TRANSFER_FEE: i64 = 8_000_000;

pub struct Fsm {
    pub db: Addr<DbExecutor>,
    pub wallet: Wallet,
    pub pool: Pool<ConnectionManager<PgConnection>>,
}

impl Actor for Fsm {
    type Context = Context<Self>;
}

#[derive(Debug, Deserialize)]
pub struct CreatePayment {
    pub merchant_id: String,
    pub external_id: String,
    pub amount: Money,
    pub confirmations: i32,
    pub email: Option<String>,
    pub message: String,
}

impl Message for CreatePayment {
    type Result = Result<UnpaidPayment, Error>;
}

impl Handler<CreatePayment> for Fsm {
    type Result = ResponseFuture<UnpaidPayment, Error>;

    fn handle(&mut self, msg: CreatePayment, _: &mut Self::Context) -> Self::Result {
        let create_transaction = CreateTransaction {
            merchant_id: msg.merchant_id,
            external_id: msg.external_id,
            amount: msg.amount,
            confirmations: msg.confirmations,
            email: msg.email.clone(),
            message: msg.message.clone(),
        };

        let res = self
            .db
            .send(create_transaction)
            .from_err()
            .and_then(move |db_response| {
                let transaction = db_response?;
                Ok(UnpaidPayment(transaction))
            });
        Box::new(res)
    }
}

#[derive(Debug, Deserialize)]
pub struct GetUnpaidPayment {
    pub transaction_id: Uuid,
}

impl Message for GetUnpaidPayment {
    type Result = Result<UnpaidPayment, Error>;
}

#[derive(Debug, Serialize, Deserialize, Clone, Deref)]
pub struct UnpaidPayment(Transaction);

impl Handler<GetUnpaidPayment> for Fsm {
    type Result = ResponseFuture<UnpaidPayment, Error>;

    fn handle(&mut self, msg: GetUnpaidPayment, _: &mut Self::Context) -> Self::Result {
        let res = self
            .db
            .send(GetTransaction {
                transaction_id: msg.transaction_id,
            })
            .from_err()
            .and_then(move |db_response| {
                let transaction = db_response?;
                if transaction.status != TransactionStatus::Unpaid {
                    return Err(Error::WrongTransactionStatus(s!(transaction.status)));
                }
                Ok(UnpaidPayment(transaction))
            });
        Box::new(res)
    }
}

#[derive(Debug, Deserialize)]
pub struct MakePayment {
    pub unpaid_payment: UnpaidPayment,
    pub wallet_tx: TxLogEntry,
}

impl Message for MakePayment {
    type Result = Result<PendingPayment, Error>;
}

impl Handler<MakePayment> for Fsm {
    type Result = ResponseFuture<PendingPayment, Error>;

    fn handle(&mut self, msg: MakePayment, _: &mut Self::Context) -> Self::Result {
        let transaction_id = msg.unpaid_payment.id.clone();
        let wallet_tx = msg.wallet_tx.clone();
        let messages: Option<Vec<String>> = wallet_tx.messages.map(|pm| {
            pm.messages
                .into_iter()
                .map(|pmd| pmd.message)
                .filter_map(|x| x)
                .collect()
        });

        let msg = UpdateTransactionWithTxLog {
            transaction_id: transaction_id.clone(),
            wallet_tx_id: wallet_tx.id as i64,
            wallet_tx_slate_id: wallet_tx.tx_slate_id.unwrap(),
            messages: messages,
        };
        let res = self
            .db
            .send(msg)
            .from_err()
            .and_then(|db_response| {
                db_response?;
                Ok(())
            })
            .and_then({
                let db = self.db.clone();
                let transaction_id = transaction_id.clone();
                move |_| {
                    db.send(UpdateTransactionStatus {
                        id: transaction_id,
                        status: TransactionStatus::Pending,
                    })
                    .from_err()
                }
            })
            .and_then(|db_response| {
                let transaction = db_response?;
                Ok(PendingPayment(transaction))
            });
        Box::new(res)
    }
}

#[derive(Debug, Deserialize)]
pub struct GetPendingPayments;

impl Message for GetPendingPayments {
    type Result = Result<Vec<PendingPayment>, Error>;
}

#[derive(Debug, Deserialize, Clone, Deref)]
pub struct PendingPayment(Transaction);

impl Handler<GetPendingPayments> for Fsm {
    type Result = ResponseFuture<Vec<PendingPayment>, Error>;

    fn handle(&mut self, _: GetPendingPayments, _: &mut Self::Context) -> Self::Result {
        Box::new(
            self.db
                .send(db::GetPendingTransactions)
                .from_err()
                .and_then(|db_response| {
                    let data = db_response?;
                    Ok(data.into_iter().map(PendingPayment).collect())
                }),
        )
    }
}

#[derive(Debug, Deserialize)]
pub struct ConfirmPayment {
    pub payment: PendingPayment,
    pub wallet_tx: TxLogEntry,
}

impl Message for ConfirmPayment {
    type Result = Result<ConfirmedPayment, Error>;
}

impl Handler<ConfirmPayment> for Fsm {
    type Result = ResponseFuture<ConfirmedPayment, Error>;

    fn handle(&mut self, msg: ConfirmPayment, _: &mut Self::Context) -> Self::Result {
        let tx_msg = db::ConfirmTransaction {
            transaction: msg.payment.0,
            confirmed_at: msg.wallet_tx.confirmation_ts.map(|dt| dt.naive_utc()),
        };
        Box::new(self.db.send(tx_msg).from_err().and_then(|res| {
            let tx = res?;
            Ok(ConfirmedPayment(tx))
        }))
    }
}

#[derive(Debug, Deserialize)]
pub struct GetConfirmedPayments;

impl Message for GetConfirmedPayments {
    type Result = Result<Vec<ConfirmedPayment>, Error>;
}

#[derive(Debug, Deserialize, Clone, Deref, Serialize)]
pub struct ConfirmedPayment(Transaction);

impl Handler<GetConfirmedPayments> for Fsm {
    type Result = ResponseFuture<Vec<ConfirmedPayment>, Error>;

    fn handle(&mut self, _: GetConfirmedPayments, _: &mut Self::Context) -> Self::Result {
        Box::new(
            self.db
                .send(db::GetConfirmedTransactions)
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
    client::post(callback_url) // <- Create request builder
        .bearer_token(token)
        .json(transaction)
        .unwrap()
        .send() // <- Send http request
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
                // <- server http response

                debug!("Response: {:?}", resp);
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

#[derive(Debug, Deserialize, Deref)]
pub struct RejectPayment<T> {
    pub payment: T,
}

impl Message for RejectPayment<UnpaidPayment> {
    type Result = Result<RejectedPayment, Error>;
}

impl Message for RejectPayment<PendingPayment> {
    type Result = Result<RejectedPayment, Error>;
}

impl Handler<RejectPayment<UnpaidPayment>> for Fsm {
    type Result = ResponseFuture<RejectedPayment, Error>;

    fn handle(&mut self, msg: RejectPayment<UnpaidPayment>, _: &mut Self::Context) -> Self::Result {
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

pub struct RejectedPayment(Transaction);

#[derive(Debug, Deserialize)]
pub struct GetUnreportedPayments;

impl Message for GetUnreportedPayments {
    type Result = Result<Vec<UnreportedPayment>, Error>;
}

#[derive(Debug, Deserialize, Clone, Deref)]
pub struct UnreportedPayment(Transaction);

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

#[derive(Debug, Deserialize)]
pub struct CreatePayout {
    pub merchant_id: String,
    pub amount: i64,
    pub confirmations: i32,
}

impl Message for CreatePayout {
    type Result = Result<UnpaidPayout, Error>;
}

#[derive(Debug, Deserialize, Clone, Deref)]
pub struct UnpaidPayout(Transaction);

impl Handler<CreatePayout> for Fsm {
    type Result = ResponseFuture<UnpaidPayout, Error>;

    fn handle(&mut self, msg: CreatePayout, _: &mut Self::Context) -> Self::Result {
        let res = blocking::run({
            let pool = self.pool.clone();
            let merchant_id = msg.merchant_id.clone();
            move || {
                use crate::schema::merchants::dsl::*;
                let conn: &PgConnection = &pool.get().unwrap();
                let merchant: Merchant = merchants.find(merchant_id.clone()).get_result(conn)?;
                if merchant.balance < msg.amount {
                    return Err(Error::NotEnoughFunds);
                }

                let transfer_fee = TRANSFER_FEE;
                let knockturn_fee = msg.amount * KNOCKTURN_FEE;
                let mut total = 0;

                if msg.amount > transfer_fee + knockturn_fee {
                    total = msg.amount - transfer_fee - knockturn_fee;
                }

                let amount = Money::from_grin(msg.amount);
                let new_transaction = Transaction {
                    id: uuid::Uuid::new_v4(),
                    external_id: s!(""),
                    merchant_id: merchant_id.clone(),
                    email: Some(merchant.email.clone()),
                    amount: amount,
                    grin_amount: -msg.amount,
                    status: TransactionStatus::Unpaid,
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
                    transfer_fee: Some(transfer_fee),
                    knockturn_fee: Some(knockturn_fee),
                    real_transfer_fee: None,
                };

                use crate::schema::transactions;
                let tx = diesel::insert_into(transactions::table)
                    .values(&new_transaction)
                    .get_result::<Transaction>(conn)?;

                Ok(UnpaidPayout(tx))
            }
        })
        .from_err();

        Box::new(res)

        /*

        Box::new(
            self.db
                .send(GetMerchant {
                    id: transaction.merchant_id.clone(),
                })
                .from_err()
                .and_then(|res| {
                    let merchant = res?;
                    Ok(merchant)
                })
                .and_then({
                    let db = self.db.clone();
                    move |merchant| {
                        if merchant.balance < msg.amount {
                            return err(Error::NotEnoughFunds);
                        }
                        db.send(db::CreateTransaction {
                            merchant_id: msg.merchant_id.clone(),
                            external_id: s!(""),
                            amount: msg.amount.clone(),
                            confirmations: msg.confirmations,
                            email: merchant.email,
                            message: format!(
                                "Withdrawal of {} for merchant {}",
                                msg.amount.clone(),
                                merchant_id.clone()
                            ),
                        })
                        .from_err()
                        .and_then(|db_response| {
                            let transaction = db_response?;
                            Ok(UnpaidPayout(transaction))
                        })
                    }
                })
                .from_err()
                .and_then(|db_response| {
                    let data = db_response?;
                    Ok(data.into_iter().map(UnreportedPayment).collect())
                }),
        )
        */
    }
}
