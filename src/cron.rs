use crate::rates;
use actix::prelude::*;

pub struct Cron {
    db: Addr<DbExecutor>,
}

impl Actor for RatesFetcher {
    type Context = Context<Self>;

    fn started(&mut self, ctx: &mut Self::Context) {
        ctx.run_interval(
            std::time::Duration::new(5, 0),
            |instance: &mut RatesFetcher, _ctx: &mut Context<Self>| {
                instance.do_send(FetchRates {}).map_err(|e| {
                    error!("failed to send rates db updte: {:?}", e);
                    ()
                })
            },
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
