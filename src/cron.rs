use crate::db::{DbExecutor, GetReceivedOrders, UpdateOrderStatus, UpdateTx};
use crate::errors::Error;
use crate::models::OrderStatus;
use crate::rates::RatesFetcher;
use crate::wallet::Wallet;
use actix::prelude::*;
use futures::future::{join_all, ok, Either, Future};

pub struct Cron {
    db: Addr<DbExecutor>,
    wallet: Wallet,
}

impl Actor for Cron {
    type Context = Context<Self>;

    fn started(&mut self, ctx: &mut Self::Context) {
        let rates = RatesFetcher::new(self.db.clone());
        ctx.run_interval(
            std::time::Duration::new(5, 0),
            move |_instance: &mut Cron, _ctx: &mut Context<Self>| {
                rates.fetch();
            },
        );
        ctx.run_interval(std::time::Duration::new(5, 0), process_orders);
    }

    fn stopping(&mut self, _ctx: &mut Self::Context) -> Running {
        Running::Stop
    }
}

impl Cron {
    pub fn new(db: Addr<DbExecutor>, wallet: Wallet) -> Self {
        Cron { db, wallet }
    }
}

fn process_orders(cron: &mut Cron, ctx: &mut Context<Cron>) {
    println!("hello");
    let wallet = cron.wallet.clone();
    let db_clone = cron.db.clone();
    let res = cron
        .db
        .send(GetReceivedOrders)
        .map_err(|e| Error::General(s!("error")))
        .and_then(move |db_response| {
            //let z: Result<(), _> = db_response;
            let orders = db_response?;
            Ok(orders)
        })
        .and_then(move |orders| {
            let mut futures = vec![];
            for (order, txs) in orders {
                println!("\x1B[31;1m order\x1B[0m = {:?}", order);
                println!("\x1B[31;1m txs\x1B[0m = {:?}", txs);
                for tx in txs {
                    println!("\x1B[31;1m tx\x1B[0m = {:?}", tx);
                    let db = db_clone.clone();
                    let db2 = db_clone.clone();
                    let order_id = order.id.clone();
                    let order_status = order.status.clone();
                    let res = wallet.get_tx(&tx.slate_id).and_then(move |wallet_tx| {
                        if wallet_tx.confirmed {
                            let mut msg = UpdateTx::from(tx);
                            msg.confirmed = true;
                            msg.confirmed_at = wallet_tx.confirmation_ts.map(|dt| dt.naive_utc());
                            let res = db
                                .send(msg)
                                .from_err()
                                .and_then(|res| {
                                    res?;
                                    Ok(())
                                })
                                .and_then(move |_| {
                                    db2.send(UpdateOrderStatus {
                                        id: order_id,
                                        status: OrderStatus::Confirmed,
                                    })
                                    .from_err()
                                    .and_then(|db_response| {
                                        db_response?;
                                        Ok(())
                                    })
                                })
                                .map(|_| ());
                            Either::A(res)
                        } else {
                            Either::B(ok(()))
                        }
                    });
                    futures.push(res);
                }
            }
            join_all(futures).map(|_| ())
            //Ok(())
        });
    //ctx.spawn(res.into_actor());
    actix::spawn(res.map_err(|e| ()));
}
