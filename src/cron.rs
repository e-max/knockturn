use crate::db::{DbExecutor, GetUnpaidOrders};
use crate::errors::Error;
use crate::rates::RatesFetcher;
use crate::wallet::Wallet;
use actix::prelude::*;
use futures::future::Future;

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
    let res = cron
        .db
        .send(GetUnpaidOrders)
        .map_err(|e| Error::General(s!("error")))
        .and_then(|db_response| {
            //let z: Result<(), _> = db_response;
            let orders = db_response?;
            //  let futures = vec![];
            for (order, txs) in orders {
                println!("\x1B[31;1m order\x1B[0m = {:?}", order);
                println!("\x1B[31;1m txs\x1B[0m = {:?}", txs);
                //self.wallet
                //.get_tx(order.slate_id)
                //.and_then(|tx| if tx.confirmed {});
            }
            Ok(())
        });
    //ctx.spawn(res.into_actor());
    actix::spawn(res.map_err(|e| ()));
}
