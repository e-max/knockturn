use crate::db::DbExecutor;
use crate::errors::Error;
use crate::fsm::{ConfirmOrder, Fsm, GetConfirmedOrders, GetPendingOrders};
use crate::models::OrderStatus;
use crate::rates::RatesFetcher;
use crate::wallet::Wallet;
use actix::prelude::*;
use futures::future::{join_all, ok, Either, Future};

pub struct Cron {
    db: Addr<DbExecutor>,
    wallet: Wallet,
    fsm: Addr<Fsm>,
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
        ctx.run_interval(std::time::Duration::new(5, 0), process_pending_orders);
        ctx.run_interval(std::time::Duration::new(5, 0), process_confirmed_orders);
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
            for (order, txs) in orders {
                println!("\x1B[31;1m order\x1B[0m = {:?}", order);
                println!("\x1B[31;1m txs\x1B[0m = {:?}", txs);
                for tx in txs {
                    println!("\x1B[31;1m tx\x1B[0m = {:?}", tx);
                    let res = wallet.get_tx(&tx.slate_id).and_then({
                        let fsm = fsm.clone();
                        let order = order.clone();
                        move |wallet_tx| {
                            println!("\x1B[31;1m wallet_tx\x1B[0m = {:?}", wallet_tx);
                            if wallet_tx.confirmed {
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
fn process_confirmed_orders(cron: &mut Cron, ctx: &mut Context<Cron>) {
    let res = cron
        .fsm
        .send(GetConfirmedOrders)
        .map_err(|e| Error::General(s!("error")))
        .and_then(move |db_response| {
            //let z: Result<(), _> = db_response;
            let orders = db_response?;
            Ok(orders)
        })
        .and_then(|confirmed_orders| {
            for (confirmed_order, merchant) in confirmed_orders {
                println!("\x1B[31;1m confirmed_order\x1B[0m = {:?}", confirmed_order);
            }
            ok(())
        });

    actix::spawn(res.map_err(|e| ()));
}
