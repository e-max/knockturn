use crate::db::DbExecutor;
use crate::rates::{FetchRates, RatesFetcher};
use actix::prelude::*;
use log::error;

pub struct Cron {
    db: Addr<DbExecutor>,
}

impl Actor for Cron {
    type Context = Context<Self>;

    fn started(&mut self, ctx: &mut Self::Context) {
        let rates = RatesFetcher::new(self.db.clone());
        ctx.run_interval(
            std::time::Duration::new(5, 0),
            move |_instance: &mut Cron, ctx: &mut Context<Self>| {
				let f = rates.fetch();
				actix::spawn(f);
			}
			//rates_addr.do_send(FetchRates {}),
        );
    }

    fn stopping(&mut self, _ctx: &mut Self::Context) -> Running {
        Running::Stop
    }
}

impl Cron {
    pub fn new(db: Addr<DbExecutor>) -> Self {
        Cron { db }
    }
}
