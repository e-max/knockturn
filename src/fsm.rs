use crate::db;
use crate::db::{ConfirmTx, CreateTx, DbExecutor, GetOrder, UpdateOrderStatus};
use crate::errors::Error;
use crate::models::OrderStatus;
use crate::models::{Order, Tx};
use crate::wallet::TxLogEntry;
use crate::wallet::Wallet;
use actix::{Actor, Addr, Context, Handler, Message, ResponseActFuture, ResponseFuture};
use futures::future::{join_all, ok, Either, Future};
use serde::Deserialize;
use std::ops::Deref;
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
    pub id: Uuid,
}

impl Message for GetUnpaidOrder {
    type Result = Result<UnpaidOrder, Error>;
}

#[derive(Debug, Deserialize, Clone)]
pub struct UnpaidOrder(Order);

impl Deref for UnpaidOrder {
    type Target = Order;

    fn deref(&self) -> &Order {
        &self.0
    }
}

impl Handler<GetUnpaidOrder> for Fsm {
    type Result = ResponseFuture<UnpaidOrder, Error>;

    fn handle(&mut self, msg: GetUnpaidOrder, _: &mut Self::Context) -> Self::Result {
        let id = msg.id.clone();
        let res = self
            .db
            .send(GetOrder { id: msg.id })
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

#[derive(Debug, Deserialize, Clone)]
pub struct PendingOrder(Order);

impl Deref for PendingOrder {
    type Target = Order;

    fn deref(&self) -> &Order {
        &self.0
    }
}

impl Handler<GetPendingOrders> for Fsm {
    //type Result = Result<Vec<(PendingOrder, Vec<Tx>)>, Error>;
    type Result = ResponseFuture<Vec<(PendingOrder, Vec<Tx>)>, Error>;

    fn handle(&mut self, msg: GetPendingOrders, _: &mut Self::Context) -> Self::Result {
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
