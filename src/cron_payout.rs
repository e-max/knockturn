use crate::errors::Error;
use crate::fsm_payout::{
    FsmPayout, GetExpiredInitializedPayouts, GetExpiredNewPayouts, GetPendingPayouts, RejectPayout,
};
use crate::rates::RatesFetcher;
use crate::Pool;
use actix::prelude::*;
use futures::future::{join_all, Future, FutureExt};
use log::*;

#[derive(Clone)]
pub struct CronPayout {
    fsm: Addr<FsmPayout>,
    pool: Pool,
}

impl CronPayout {
    pub fn new(fsm: Addr<FsmPayout>, pool: Pool) -> Self {
        CronPayout { fsm, pool }
    }
    async fn process_expired_new_payouts(&self) -> Result<(), Error> {
        debug!("run process_expired_new_payouts");
        let payouts = self
            .fsm
            .send(GetExpiredNewPayouts)
            .await
            .map_err(|e| Error::General(s!(e)))??;
        debug!("Found {} expired new payouts", payouts.len());
        for payout in payouts {
            let res: Result<(), Error> = self
                .fsm
                .send(RejectPayout { payout })
                .await
                .map_err(|e| Error::General(s!(e)))
                .and_then(|db_response| {
                    db_response?;
                    Ok(())
                })
                .or_else({
                    move |e| {
                        error!("Couldn't reject payout {}: {}", payout.id, e);
                        Ok(())
                    }
                });
            res?;
        }

        Ok(())
    }

    async fn process_expired_initialized_payouts(&self) -> Result<(), Error> {
        debug!("run process_expired_initialized_payouts");
        let payouts = self
            .fsm
            .send(GetExpiredInitializedPayouts)
            .await
            .map_err(|e| Error::General(s!(e)))??;
        debug!("Found {} expired initialized payouts", payouts.len());
        for payout in payouts {
            let res: Result<(), Error> = self
                .fsm
                .send(RejectPayout { payout })
                .await
                .map_err(|e| Error::General(s!(e)))
                .and_then(|db_response| {
                    db_response?;
                    Ok(())
                })
                .or_else({
                    move |e| {
                        error!("Couldn't reject payout {}: {}", payout.id, e);
                        Ok(())
                    }
                });
            res?;
        }

        Ok(())
    }
    async fn process_pending_payouts(&self) -> Result<(), Error> {
        debug!("run process_pending_payouts");

        let payouts = self
            .fsm
            .send(GetPendingPayouts)
            .await
            .map_err(|e| Error::General(s!(e)))??;
        debug!("Found {} pending payouts", payouts.len());
        for payout in payouts {
            let res: Result<(), Error> = self
                .fsm
                .send(RejectPayout {
                    payout: payout.clone(),
                })
                .await
                .map_err(|e| Error::General(s!(e)))
                .and_then(|db_response| {
                    db_response?;
                    Ok(())
                })
                .or_else(move |e| {
                    error!("Cannot reject payout {}: {}", payout.id, e);
                    Ok(())
                });
            res?;
        }
        Ok(())
    }
}

impl Actor for CronPayout {
    type Context = Context<Self>;

    fn started(&mut self, ctx: &mut Self::Context) {
        info!("Starting cron process");
        let rates = RatesFetcher::new(self.pool.clone());
        ctx.run_interval(
            std::time::Duration::new(5, 0),
            move |_instance: &mut CronPayout, _ctx: &mut Context<Self>| {
                rates.fetch();
            },
        );
        ctx.run_interval(std::time::Duration::new(5, 0), process_expired_new_payouts);
        ctx.run_interval(
            std::time::Duration::new(5, 0),
            process_expired_initialized_payouts,
        );
        ctx.run_interval(std::time::Duration::new(5, 0), process_pending_payouts);
    }

    fn stopping(&mut self, _ctx: &mut Self::Context) -> Running {
        Running::Stop
    }
}

fn process_expired_new_payouts(cron: &mut CronPayout, _: &mut Context<CronPayout>) {
    let cron = cron.clone();
    actix::spawn(cron.process_expired_new_payouts().map(|_| ()));
}

fn process_expired_initialized_payouts(cron: &mut CronPayout, _: &mut Context<CronPayout>) {
    let cron = cron.clone();
    actix::spawn(cron.process_expired_initialized_payouts().map(|_| ()));
}

fn process_pending_payouts(cron: &mut CronPayout, _: &mut Context<CronPayout>) {
    let cron = cron.clone();
    actix::spawn(cron.process_pending_payouts().map(|_| ()));
}
