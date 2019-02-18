use crate::db::DbExecutor;
use actix::prelude::*;

pub struct RatesFetcher {
    db: Addr<DbExecutor>,
}

impl Actor for RatesFetcher {
    type Context = Context<Self>;

    fn started(&mut self, ctx: &mut Self::Context) {
        ctx.run_interval(
            std::time::Duration::new(5, 0),
            |instance: &mut RatesFetcher, _ctx: &mut Context<Self>| instance.fetch(),
        );
    }

    fn stopping(&mut self, _ctx: &mut Self::Context) -> Running {
        Running::Stop
    }
}

impl RatesFetcher {
    pub fn new(db: Addr<DbExecutor>) -> Self {
        RatesFetcher { db }
    }
    fn fetch(&mut self) {
        println!("fetching");
    }
}
