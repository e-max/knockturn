use crate::clients::BearerTokenAuth;
use crate::db::DbExecutor;
use crate::errors::Error;
use crate::fsm::{
    ConfirmOrder, Fsm, GetConfirmedOrders, GetPendingOrders, GetUnreportedOrders, RejectOrder,
    ReportOrder,
};
use crate::models::OrderStatus;
use crate::rates::RatesFetcher;
use crate::wallet::Wallet;
use actix::prelude::*;
use actix_web::client;
use futures::future::{join_all, ok, Either, Future};
use log::*;

pub struct Cron {
    db: Addr<DbExecutor>,
    wallet: Wallet,
    fsm: Addr<Fsm>,
}

impl Actor for Cron {
    type Context = Context<Self>;

    fn started(&mut self, ctx: &mut Self::Context) {
        info!("Starting cron process");
        let rates = RatesFetcher::new(self.db.clone());
        ctx.run_interval(
            std::time::Duration::new(5, 0),
            move |_instance: &mut Cron, _ctx: &mut Context<Self>| {
                rates.fetch();
            },
        );
        ctx.run_interval(std::time::Duration::new(5, 0), process_pending_orders);
        ctx.run_interval(std::time::Duration::new(5, 0), process_unreported_orders);
    }

    fn stopping(&mut self, _ctx: &mut Self::Context) -> Running {
        Running::Stop
    }
}

impl Cron {
    pub fn new(db: Addr<DbExecutor>, wallet: Wallet, fsm: Addr<Fsm>) -> Self {
        Cron { db, wallet, fsm }
    }
}

fn process_pending_orders(cron: &mut Cron, ctx: &mut Context<Cron>) {
    println!("hello");
    debug!("run process_pending_orders");
    let wallet = cron.wallet.clone();
    let db_clone = cron.db.clone();
    let fsm = cron.fsm.clone();
    let res = cron
        .fsm
        .send(GetPendingOrders)
        .map_err(|e| Error::General(s!("error")))
        .and_then(move |db_response| {
            //let z: Result<(), _> = db_response;
            let orders = db_response?;
            Ok(orders)
        })
        .and_then(move |orders| {
            let mut futures = vec![];
            debug!("Found {} pending orders", orders.len());
            for (order, txs) in orders {
                if order.is_expired() {
                    debug!("Order {} expired: try to reject it", order.id);
                    futures.push(Either::A(
                        fsm.send(RejectOrder {
                            order: order.clone(),
                        })
                        .map_err(|e| Error::General(s!(e)))
                        .and_then(|db_response| {
                            db_response?;
                            Ok(())
                        })
                        .or_else({
                            let order_id = order.id.clone();
                            move |e| {
                                error!("Cannot reject order {}: {}", order.id, e);
                                Ok(())
                            }
                        }),
                    ));
                    continue;
                }
                println!("\x1B[31;1m order\x1B[0m = {:?}", order);
                println!("\x1B[31;1m txs\x1B[0m = {:?}", txs);
                for tx in txs {
                    println!("\x1B[31;1m tx\x1B[0m = {:?}", tx);
                    let res = wallet
                        .get_tx(&tx.slate_id)
                        .and_then({
                            let fsm = fsm.clone();
                            let order = order.clone();
                            move |wallet_tx| {
                                println!("\x1B[31;1m wallet_tx\x1B[0m = {:?}", wallet_tx);
                                if wallet_tx.confirmed {
                                    info!("Order {} confirmed", order.id);
                                    let res = fsm
                                        .send(ConfirmOrder { order, wallet_tx })
                                        .from_err()
                                        .and_then(|msg_response| {
                                            msg_response?;
                                            Ok(())
                                        });
                                    Either::A(res)
                                } else {
                                    Either::B(ok(()))
                                }
                            }
                        })
                        .or_else({
                            let order_id = order.id.clone();
                            move |e| {
                                warn!("Couldn't confirm order {}: {}", order_id, e);
                                Ok(())
                            }
                        });
                    futures.push(Either::B(res));
                }
            }
            join_all(futures).map(|_| ())
            //Ok(())
        });
    //ctx.spawn(res.into_actor());
    actix::spawn(res.map_err(|e| ()));
}
fn process_unreported_orders(cron: &mut Cron, ctx: &mut Context<Cron>) {
    let res = cron
        .fsm
        .send(GetUnreportedOrders)
        .map_err(|e| Error::General(s!("error")))
        .and_then(move |db_response| {
            let orders = db_response?;
            Ok(orders)
        })
        .and_then({
            let fsm = cron.fsm.clone();
            move |orders| {
                let mut futures = vec![];
                debug!("Found {} unreported orders", orders.len());
                for order in orders {
                    let order_id = order.id.clone();
                    futures.push(
                        fsm.send(ReportOrder { order })
                            .map_err(|e| Error::General(s!(e)))
                            .and_then(|db_response| {
                                db_response?;
                                Ok(())
                            })
                            .or_else({
                                move |e| {
                                    warn!("Couldn't report order {}: {}", order_id, e);
                                    Ok(())
                                }
                            }),
                    );
                }
                join_all(futures).map(|_| ()).map_err(|e| {
                    error!("got an error {}", e);
                    e
                })
            }
        });

    actix::spawn(res.map_err(|e| {
        error!("got an error {}", e);
        ()
    }));
}
