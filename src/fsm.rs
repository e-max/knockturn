use crate::db;
use crate::db::{ConfirmTx, DbExecutor, UpdateOrderStatus};
use crate::errors::Error;
use crate::models::OrderStatus;
use crate::models::{Order, Tx};
use crate::wallet::TxLogEntry;
use crate::wallet::Wallet;
use actix::{Actor, Addr, Context, Handler, Message, ResponseActFuture, ResponseFuture};
use futures::future::{join_all, ok, Either, Future};
use serde::Deserialize;
use std::ops::Deref;

pub struct Fsm {
    pub db: Addr<DbExecutor>,
    pub wallet: Wallet,
}

impl Actor for Fsm {
    type Context = Context<Self>;
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
        let id = msg.order.id.clone();
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
