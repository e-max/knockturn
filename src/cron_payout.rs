use crate::errors::Error;
use crate::fsm_payout::{
    FsmPayout, GetExpiredInitializedPayouts, GetExpiredNewPayouts, GetPendingPayouts, RejectPayout,
};
use crate::rates::RatesFetcher;
use actix::prelude::*;
use diesel::pg::PgConnection;
use diesel::r2d2::{ConnectionManager, Pool};
use futures::future::{join_all, Future};
use log::*;

pub struct CronPayout {
    fsm: Addr<FsmPayout>,
    pool: Pool<ConnectionManager<PgConnection>>,
}

impl CronPayout {
    pub fn new(fsm: Addr<FsmPayout>, pool: Pool<ConnectionManager<PgConnection>>) -> Self {
        CronPayout { fsm, pool }
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
    debug!("run process_expired_new_payouts");
    let res = cron
        .fsm
        .send(GetExpiredNewPayouts)
        .map_err(|e| Error::General(s!(e)))
        .and_then(|db_response| {
            let payouts = db_response?;
            Ok(payouts)
        })
        .and_then({
            let fsm = cron.fsm.clone();
            move |payouts| {
                let mut futures = vec![];
                debug!("Found {} expired new payouts", payouts.len());
                for payout in payouts {
                    let payout_id = payout.id.clone();
                    futures.push(
                        fsm.send(RejectPayout { payout })
                            .map_err(|e| Error::General(s!(e)))
                            .and_then(|db_response| {
                                db_response?;
                                Ok(())
                            })
                            .or_else({
                                move |e| {
                                    error!("Couldn't reject payout {}: {}", payout_id, e);
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
    actix::spawn(res.map_err(|e| error!("Got an error in processing expired new payouts {}", e)));
}

fn process_expired_initialized_payouts(cron: &mut CronPayout, _: &mut Context<CronPayout>) {
    debug!("run process_expired_initialized_payouts");
    let res = cron
        .fsm
        .send(GetExpiredInitializedPayouts)
        .map_err(|e| Error::General(s!(e)))
        .and_then(|db_response| {
            let payouts = db_response?;
            Ok(payouts)
        })
        .and_then({
            let fsm = cron.fsm.clone();
            move |payouts| {
                let mut futures = vec![];
                debug!("Found {} expired initialized payouts", payouts.len());
                for payout in payouts {
                    let payout_id = payout.id.clone();
                    futures.push(
                        fsm.send(RejectPayout { payout })
                            .map_err(|e| Error::General(s!(e)))
                            .and_then(|db_response| {
                                db_response?;
                                Ok(())
                            })
                            .or_else({
                                move |e| {
                                    error!("Couldn't reject payout {}: {}", payout_id, e);
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
        error!(
            "Got an error in processing expired initialized payouts {}",
            e
        )
    }));
}

fn process_pending_payouts(cron: &mut CronPayout, _: &mut Context<CronPayout>) {
    debug!("run process_pending_payouts");
    let fsm = cron.fsm.clone();
    let res = cron
        .fsm
        .send(GetPendingPayouts)
        .map_err(|e| Error::General(s!(e)))
        .and_then(move |db_response| {
            let payouts = db_response?;
            Ok(payouts)
        })
        .and_then(move |payouts| {
            let mut futures = vec![];
            debug!("Found {} pending payouts", payouts.len());
            for payout in payouts {
                if payout.is_expired() {
                    debug!("payout {} expired: try to reject it", payout.id);
                    futures.push(
                        fsm.send(RejectPayout {
                            payout: payout.clone(),
                        })
                        .map_err(|e| Error::General(s!(e)))
                        .and_then(|db_response| {
                            db_response?;
                            Ok(())
                        })
                        .or_else(move |e| {
                            error!("Cannot reject payout {}: {}", payout.id, e);
                            Ok(())
                        }),
                    );
                }
            }
            join_all(futures).map(|_| ())
        });
    actix::spawn(res.map_err(|e| error!("Got an error in processing penging payouts {}", e)));
}
