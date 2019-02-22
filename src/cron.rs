use crate::db::{DbExecutor, GetUnpaidOrders, UpdateTx};
use crate::errors::Error;
use crate::rates::RatesFetcher;
use crate::wallet::Wallet;
use actix::prelude::*;
use futures::future::{join_all, Either, Future};

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
    let db = cron.db.clone();
    let res = cron
        .db
        .send(GetUnpaidOrders)
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
                    /*
                    let res = wallet.get_tx(&tx.slate_id).and_then(|wallet_tx| {
                        if tx.confirmed {
                            let mut msg = UpdateTx::from(tx);
                            msg.confirmed = true;
                            if let Some(confirmation_time) = wallet_tx.confirmation_ts {
                                msg.confirmed_at = confirmation_time.naive_utc();
                            }
                            Either::A(
                                db.send(msg)
                                    .map_err(|e| Error::General(s!("Cannot send message")))
                                    .and_then(|resp| resp?),
                            )
                        } else {
                            Either::B(())
                        }
                    });
                    */
                    let res = wallet.get_tx(&tx.slate_id);
                    futures.push(res);
                }
            }
            join_all(futures).map(|_| ())
            //Ok(())
        });
    //ctx.spawn(res.into_actor());
    actix::spawn(res.map_err(|e| ()));
}
