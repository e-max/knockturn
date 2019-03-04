use crate::clients::BearerTokenAuth;
use crate::db;
use crate::db::{
    ConfirmTx, CreateTx, DbExecutor, GetMerchant, GetOrder, MarkAsReported, ReportAttempt,
    UpdateOrderStatus,
};
use crate::errors::Error;
use crate::models::{Order, OrderStatus, Tx};
use crate::wallet::TxLogEntry;
use crate::wallet::Wallet;
use actix::{Actor, Addr, Context, Handler, Message, ResponseFuture};
use actix_web::client;
use chrono::{Duration, Utc};
use derive_deref::Deref;
use futures::future::{Either, Future};
use log::{debug, error};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

pub struct Fsm {
    pub db: Addr<DbExecutor>,
    pub wallet: Wallet,
}

impl Actor for Fsm {
    type Context = Context<Self>;
}

#[derive(Debug, Deserialize)]
pub struct GetUnpaidOrder {
    pub order_id: Uuid,
}

impl Message for GetUnpaidOrder {
    type Result = Result<UnpaidOrder, Error>;
}

#[derive(Debug, Deserialize, Clone, Deref)]
pub struct UnpaidOrder(Order);

impl Handler<GetUnpaidOrder> for Fsm {
    type Result = ResponseFuture<UnpaidOrder, Error>;

    fn handle(&mut self, msg: GetUnpaidOrder, _: &mut Self::Context) -> Self::Result {
        let res = self
            .db
            .send(GetOrder {
                order_id: msg.order_id,
            })
            .from_err()
            .and_then(move |db_response| {
                let order = db_response?;
                if order.status != OrderStatus::Unpaid {
                    return Err(Error::WrongOrderStatus(s!(order.status)));
                }
                Ok(UnpaidOrder(order))
            });
        Box::new(res)
    }
}

#[derive(Debug, Deserialize)]
pub struct PayOrder {
    pub unpaid_order: UnpaidOrder,
    pub wallet_tx: TxLogEntry,
}

impl Message for PayOrder {
    type Result = Result<(), Error>;
}

impl Handler<PayOrder> for Fsm {
    type Result = ResponseFuture<(), Error>;

    fn handle(&mut self, msg: PayOrder, _: &mut Self::Context) -> Self::Result {
        let order_id = msg.unpaid_order.id.clone();
        let tx = msg.wallet_tx.clone();
        let messages: Vec<String> = if let Some(pm) = tx.messages {
            pm.messages
                .into_iter()
                .map(|pmd| pmd.message)
                .filter_map(|x| x)
                .collect()
        } else {
            vec![]
        };

        let msg = CreateTx {
            slate_id: tx.tx_slate_id.unwrap(),
            created_at: tx.creation_ts.naive_utc(),
            confirmed: tx.confirmed,
            confirmed_at: tx.confirmation_ts.map(|dt| dt.naive_utc()),
            fee: tx.fee.map(|f| f as i64),
            messages: messages,
            num_inputs: tx.num_inputs as i64,
            num_outputs: tx.num_outputs as i64,
            //FIXME
            tx_type: format!("{:?}", tx.tx_type),
            order_id: order_id,
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
                let order_id = order_id.clone();
                move |_| {
                    db.send(UpdateOrderStatus {
                        id: order_id,
                        status: OrderStatus::Pending,
                    })
                    .from_err()
                }
            })
            .and_then(|db_response| {
                db_response?;
                Ok(())
            });
        Box::new(res)
    }
}

#[derive(Debug, Deserialize)]
pub struct GetPendingOrders;

impl Message for GetPendingOrders {
    type Result = Result<Vec<(PendingOrder, Vec<Tx>)>, Error>;
}

#[derive(Debug, Deserialize, Clone, Deref)]
pub struct PendingOrder(Order);

impl Handler<GetPendingOrders> for Fsm {
    //type Result = Result<Vec<(PendingOrder, Vec<Tx>)>, Error>;
    type Result = ResponseFuture<Vec<(PendingOrder, Vec<Tx>)>, Error>;

    fn handle(&mut self, _: GetPendingOrders, _: &mut Self::Context) -> Self::Result {
        Box::new(
            self.db
                .send(db::GetPendingOrders)
                .from_err()
                .and_then(|db_response| {
                    let data = db_response?;
                    Ok(data
                        .into_iter()
                        .map(|(order, txs)| (PendingOrder(order), txs))
                        .collect())
                }),
        )
    }
}

#[derive(Debug, Deserialize)]
pub struct ConfirmOrder {
    pub order: PendingOrder,
    pub wallet_tx: TxLogEntry,
}

impl Message for ConfirmOrder {
    type Result = Result<(), Error>;
}

impl Handler<ConfirmOrder> for Fsm {
    type Result = ResponseFuture<(), Error>;

    fn handle(&mut self, msg: ConfirmOrder, _: &mut Self::Context) -> Self::Result {
        let tx_msg = ConfirmTx {
            slate_id: msg.wallet_tx.tx_slate_id.unwrap(),
            confirmed_at: msg.wallet_tx.confirmation_ts.map(|dt| dt.naive_utc()),
        };
        Box::new(
            self.db
                .send(tx_msg)
                .from_err()
                .and_then(|res| {
                    res?;
                    Ok(())
                })
                .and_then({
                    let db = self.db.clone();
                    let order_id = msg.order.id.clone();
                    move |_| {
                        db.send(UpdateOrderStatus {
                            id: order_id,
                            status: OrderStatus::Confirmed,
                        })
                        .from_err()
                        .and_then(|db_response| {
                            db_response?;
                            Ok(())
                        })
                    }
                }),
        )
    }
}

#[derive(Debug, Deserialize)]
pub struct GetConfirmedOrders;

impl Message for GetConfirmedOrders {
    type Result = Result<Vec<ConfirmedOrder>, Error>;
}

#[derive(Debug, Deserialize, Clone, Deref, Serialize)]
pub struct ConfirmedOrder(Order);

impl Handler<GetConfirmedOrders> for Fsm {
    type Result = ResponseFuture<Vec<ConfirmedOrder>, Error>;

    fn handle(&mut self, _: GetConfirmedOrders, _: &mut Self::Context) -> Self::Result {
        Box::new(
            self.db
                .send(db::GetConfirmedOrders)
                .from_err()
                .and_then(|db_response| {
                    let data = db_response?;
                    Ok(data.into_iter().map(ConfirmedOrder).collect())
                }),
        )
    }
}

fn run_callback(
    callback_url: &str,
    token: &String,
    order: Order,
) -> impl Future<Item = (), Error = Error> {
    client::post(callback_url) // <- Create request builder
        .bearer_token(token)
        .json(order)
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
pub struct RejectOrder<T> {
    pub order: T,
}

impl Message for RejectOrder<UnpaidOrder> {
    type Result = Result<(), Error>;
}

impl Message for RejectOrder<PendingOrder> {
    type Result = Result<(), Error>;
}

impl Handler<RejectOrder<UnpaidOrder>> for Fsm {
    type Result = ResponseFuture<(), Error>;

    fn handle(&mut self, msg: RejectOrder<UnpaidOrder>, _: &mut Self::Context) -> Self::Result {
        Box::new(reject_order(&self.db, &msg.order.id))
    }
}

impl Handler<RejectOrder<PendingOrder>> for Fsm {
    type Result = ResponseFuture<(), Error>;

    fn handle(&mut self, msg: RejectOrder<PendingOrder>, _: &mut Self::Context) -> Self::Result {
        Box::new(reject_order(&self.db, &msg.order.id))
    }
}

fn reject_order(db: &Addr<DbExecutor>, id: &Uuid) -> impl Future<Item = (), Error = Error> {
    db.send(UpdateOrderStatus {
        id: id.clone(),
        status: OrderStatus::Rejected,
    })
    .from_err()
    .and_then(|db_response| {
        db_response?;
        Ok(())
    })
}

#[derive(Debug, Deserialize, Deref)]
pub struct ReportOrder<T> {
    pub order: T,
}

impl Message for ReportOrder<ConfirmedOrder> {
    type Result = Result<(), Error>;
}

impl Message for ReportOrder<RejectedOrder> {
    type Result = Result<(), Error>;
}

impl Message for ReportOrder<UnreportedOrder> {
    type Result = Result<(), Error>;
}

impl Handler<ReportOrder<ConfirmedOrder>> for Fsm {
    type Result = ResponseFuture<(), Error>;

    fn handle(&mut self, msg: ReportOrder<ConfirmedOrder>, _: &mut Self::Context) -> Self::Result {
        Box::new(report_order(self.db.clone(), msg.order.0))
    }
}

impl Handler<ReportOrder<RejectedOrder>> for Fsm {
    type Result = ResponseFuture<(), Error>;

    fn handle(&mut self, msg: ReportOrder<RejectedOrder>, _: &mut Self::Context) -> Self::Result {
        Box::new(report_order(self.db.clone(), msg.order.0))
    }
}

impl Handler<ReportOrder<UnreportedOrder>> for Fsm {
    type Result = ResponseFuture<(), Error>;

    fn handle(&mut self, msg: ReportOrder<UnreportedOrder>, _: &mut Self::Context) -> Self::Result {
        Box::new(report_order(self.db.clone(), msg.order.0))
    }
}

fn report_order(db: Addr<DbExecutor>, order: Order) -> impl Future<Item = (), Error = Error> {
    debug!("Try to report order {}", order.id);
    db.send(GetMerchant {
        id: order.merchant_id.clone(),
    })
    .from_err()
    .and_then(|res| {
        let merchant = res?;
        Ok(merchant)
    })
    .and_then(move |merchant| {
        if let Some(callback_url) = merchant.callback_url.clone() {
            debug!("Run callback for merchant {}", merchant.email);
            let res = run_callback(&callback_url, &merchant.token, order.clone())
                .or_else({
                    let db = db.clone();
                    let order = order.clone();
                    move |callback_err| {
                        // try call ReportAttempt but ignore errors and return
                        // error from callback
                        let next_attempt = Utc::now().naive_utc()
                            + Duration::seconds(10 * (order.report_attempts + 1).pow(2) as i64);
                        db.send(ReportAttempt {
                            order_id: order.id,
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
                    let order_id = order.id.clone();
                    move |_| {
                        db.send(MarkAsReported { order_id })
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
                db.send(MarkAsReported { order_id: order.id })
                    .from_err()
                    .and_then(|db_response| {
                        db_response?;
                        Ok(())
                    }),
            )
        }
    })
}

pub struct RejectedOrder(Order);

#[derive(Debug, Deserialize)]
pub struct GetUnreportedOrders;

impl Message for GetUnreportedOrders {
    type Result = Result<Vec<UnreportedOrder>, Error>;
}

#[derive(Debug, Deserialize, Clone, Deref)]
pub struct UnreportedOrder(Order);

impl Handler<GetUnreportedOrders> for Fsm {
    type Result = ResponseFuture<Vec<UnreportedOrder>, Error>;

    fn handle(&mut self, _: GetUnreportedOrders, _: &mut Self::Context) -> Self::Result {
        Box::new(
            self.db
                .send(db::GetUnreportedOrders)
                .from_err()
                .and_then(|db_response| {
                    let data = db_response?;
                    Ok(data.into_iter().map(UnreportedOrder).collect())
                }),
        )
    }
}
