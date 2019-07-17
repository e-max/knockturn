use crate::db::{self, get_balance, update_transaction_status, DbExecutor};
use crate::errors::Error;
use crate::models::Merchant;
use crate::models::{Money, Transaction, TransactionStatus, TransactionType};
use crate::models::{NEW_PAYOUT_TTL_SECONDS, PENDING_PAYOUT_TTL_SECONDS};
use crate::ser;
use crate::wallet::TxLogEntry;
use crate::wallet::Wallet;
use crate::Pool;
use actix::{Actor, Addr, Context, Handler, Message, ResponseFuture};
use actix_web::web::block;
use chrono::{Duration, Utc};
use derive_deref::Deref;
use diesel::pg::PgConnection;
use diesel::{self, prelude::*};
use futures::future::Future;
use log::warn;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

pub const MINIMAL_WITHDRAW: i64 = 1_000_000_000;
pub const KNOCKTURN_SHARE: f64 = 0.01;
pub const TRANSFER_FEE: i64 = 8_000_000;

pub struct FsmPayout {
    pub db: Addr<DbExecutor>,
    pub wallet: Wallet,
    pub pool: Pool,
}

impl Actor for FsmPayout {
    type Context = Context<Self>;
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

impl Handler<CreatePayout> for FsmPayout {
    type Result = ResponseFuture<NewPayout, Error>;

    fn handle(&mut self, msg: CreatePayout, _: &mut Self::Context) -> Self::Result {
        let res = block::<_, _, Error>({
            let pool = self.pool.clone();
            let merchant_id = msg.merchant_id.clone();
            move || {
                use crate::schema::merchants::dsl::*;
                let conn: &PgConnection = &pool.get().unwrap();
                let tx = conn.transaction(|| {
                    let merchant: Merchant =
                        merchants.find(merchant_id.clone()).get_result(conn)?;
                    let balance = get_balance(&merchant_id, &conn)?;
                    if balance < msg.amount {
                        return Err(Error::NotEnoughFunds);
                    }

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

impl Handler<GetPayout> for FsmPayout {
    type Result = ResponseFuture<Transaction, Error>;

    fn handle(&mut self, msg: GetPayout, _: &mut Self::Context) -> Self::Result {
        let res = block::<_, _, Error>({
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

impl Handler<InitializePayout> for FsmPayout {
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

        let res = block::<_, _, Error>(move || {
            use crate::schema::transactions::dsl::*;
            let conn: &PgConnection = &pool.get().unwrap();

            let transaction = diesel::update(transactions.filter(id.eq(transaction_id.clone())))
                .set((
                    wallet_tx_id.eq(msg.wallet_tx.id as i64),
                    wallet_tx_slate_id.eq(msg.wallet_tx.tx_slate_id.unwrap()),
                    slate_messages.eq(messages),
                    real_transfer_fee.eq(msg.wallet_tx.fee.map(|fee| fee as i64)),
                    status.eq(TransactionStatus::Initialized),
                    commit.eq(ser::to_hex(msg.commit)),
                ))
                .get_result(conn)
                .map_err::<Error, _>(|e| e.into())?;
            Ok(InitializedPayout(transaction))
        })
        .from_err();

        Box::new(res)
    }
}

impl Handler<GetNewPayout> for FsmPayout {
    type Result = ResponseFuture<NewPayout, Error>;

    fn handle(&mut self, msg: GetNewPayout, _: &mut Self::Context) -> Self::Result {
        let res = block::<_, _, Error>({
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

impl Handler<GetInitializedPayout> for FsmPayout {
    type Result = ResponseFuture<InitializedPayout, Error>;

    fn handle(&mut self, msg: GetInitializedPayout, _: &mut Self::Context) -> Self::Result {
        let res = block::<_, _, Error>({
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

impl Handler<FinalizePayout> for FsmPayout {
    type Result = ResponseFuture<PendingPayout, Error>;

    fn handle(&mut self, msg: FinalizePayout, _: &mut Self::Context) -> Self::Result {
        let pool = self.pool.clone();
        Box::new(
            block::<_, _, Error>(move || {
                let conn: &PgConnection = &pool.get().unwrap();
                update_transaction_status(
                    msg.initialized_payout.id.clone(),
                    TransactionStatus::Pending,
                    &conn,
                )
                .map(PendingPayout)
            })
            .from_err(),
        )
    }
}

impl Handler<ConfirmPayout> for FsmPayout {
    type Result = ResponseFuture<ConfirmedPayout, Error>;

    fn handle(&mut self, msg: ConfirmPayout, _: &mut Self::Context) -> Self::Result {
        Box::new(
            block::<_, _, Error>({
                let pool = self.pool.clone();
                move || {
                    use crate::schema::transactions::dsl::*;
                    let conn: &PgConnection = &pool.get().unwrap();

                    let tx = diesel::update(transactions.filter(id.eq(msg.payout.0.id)))
                        .set((
                            status.eq(TransactionStatus::Confirmed),
                            updated_at.eq(Utc::now().naive_utc()),
                        ))
                        .get_result(conn)?;
                    Ok(ConfirmedPayout(tx))
                }
            })
            .from_err(),
        )
    }
}

impl Handler<GetPendingPayouts> for FsmPayout {
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

impl Handler<GetExpiredNewPayouts> for FsmPayout {
    type Result = ResponseFuture<Vec<NewPayout>, Error>;

    fn handle(&mut self, _: GetExpiredNewPayouts, _: &mut Self::Context) -> Self::Result {
        let res = block::<_, _, Error>({
            let pool = self.pool.clone();
            move || {
                use crate::schema::transactions::dsl::*;
                let conn: &PgConnection = &pool.get().unwrap();
                transactions
                    .filter(status.eq(TransactionStatus::New))
                    .filter(transaction_type.eq(TransactionType::Payout))
                    .filter(
                        created_at
                            .lt(Utc::now().naive_utc() - Duration::seconds(NEW_PAYOUT_TTL_SECONDS)),
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

impl Handler<GetExpiredInitializedPayouts> for FsmPayout {
    type Result = ResponseFuture<Vec<InitializedPayout>, Error>;

    fn handle(&mut self, _: GetExpiredInitializedPayouts, _: &mut Self::Context) -> Self::Result {
        let res = block::<_, _, Error>({
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

impl Handler<RejectPayout<NewPayout>> for FsmPayout {
    type Result = ResponseFuture<RejectedPayout, Error>;

    fn handle(&mut self, msg: RejectPayout<NewPayout>, _: &mut Self::Context) -> Self::Result {
        let res = block::<_, _, Error>({
            let pool = self.pool.clone();
            move || {
                let conn: &PgConnection = &pool.get().unwrap();
                let payout = msg.payout;
                warn!("Reject payout {:?}", payout);
                use crate::schema::transactions::dsl::*;
                let tx = diesel::update(
                    transactions
                        .filter(id.eq(payout.id.clone()))
                        .filter(status.eq(TransactionStatus::New)),
                )
                .set((
                    status.eq(TransactionStatus::Rejected),
                    updated_at.eq(Utc::now().naive_utc()),
                ))
                .get_result(conn)?;

                Ok(RejectedPayout(tx))
            }
        })
        .from_err();
        Box::new(res)
    }
}

impl Handler<RejectPayout<InitializedPayout>> for FsmPayout {
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
                    block::<_, _, Error>({
                        move || {
                            let conn: &PgConnection = &pool.get().unwrap();
                            use crate::schema::transactions::dsl::*;
                            let tx = diesel::update(
                                transactions
                                    .filter(id.eq(msg.payout.id.clone()))
                                    .filter(status.eq(TransactionStatus::Initialized)),
                            )
                            .set((
                                status.eq(TransactionStatus::Rejected),
                                updated_at.eq(Utc::now().naive_utc()),
                            ))
                            .get_result(conn)?;

                            Ok(RejectedPayout(tx))
                        }
                    })
                    .from_err()
                }
            });

        Box::new(res)
    }
}

impl Handler<RejectPayout<PendingPayout>> for FsmPayout {
    type Result = ResponseFuture<RejectedPayout, Error>;

    fn handle(&mut self, msg: RejectPayout<PendingPayout>, _: &mut Self::Context) -> Self::Result {
        let res = self
            .wallet
            .cancel_tx(&msg.payout.wallet_tx_slate_id.clone().unwrap())
            .and_then({
                let pool = self.pool.clone();
                move |_| {
                    block::<_, _, Error>({
                        move || {
                            let conn: &PgConnection = &pool.get().unwrap();
                            let payout = msg.payout;
                            warn!("Reject payout {:?}", payout);
                            use crate::schema::transactions::dsl::*;
                            let tx = diesel::update(
                                transactions
                                    .filter(id.eq(payout.id.clone()))
                                    .filter(status.eq(TransactionStatus::Pending)),
                            )
                            .set((
                                status.eq(TransactionStatus::Rejected),
                                updated_at.eq(Utc::now().naive_utc()),
                            ))
                            .get_result(conn)?;

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
