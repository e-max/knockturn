use crate::db::{DbExecutor, RejectExpiredPayments};
use crate::errors::Error;
use crate::fsm::{
    ConfirmPayment, ConfirmPayout, Fsm, GetExpiredInitializedPayouts, GetExpiredNewPayouts,
    GetPendingPayments, GetPendingPayouts, GetUnreportedPayments, RejectPayment, RejectPayout,
    ReportPayment,
};
use crate::rates::RatesFetcher;
use crate::wallet::Wallet;
use actix::prelude::*;
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
        ctx.run_interval(std::time::Duration::new(5, 0), reject_expired_payments);
        ctx.run_interval(std::time::Duration::new(5, 0), process_pending_payments);
        ctx.run_interval(std::time::Duration::new(5, 0), process_unreported_payments);
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

impl Cron {
    pub fn new(db: Addr<DbExecutor>, wallet: Wallet, fsm: Addr<Fsm>) -> Self {
        Cron { db, wallet, fsm }
    }
}
fn reject_expired_payments(cron: &mut Cron, _: &mut Context<Cron>) {
    debug!("run process_expired_payments");
    let res = cron
        .db
        .send(RejectExpiredPayments)
        .map_err(|e| Error::from(e))
        .and_then(|db_response| {
            db_response?;
            Ok(())
        });
    actix::spawn(res.map_err(|e| error!("Got an error in rejecting exprired payments {}", e)));
}

fn process_pending_payments(cron: &mut Cron, _: &mut Context<Cron>) {
    debug!("run process_pending_payments");
    let wallet = cron.wallet.clone();
    let fsm = cron.fsm.clone();
    let res = cron
        .fsm
        .send(GetPendingPayments)
        .map_err(|e| Error::General(s!(e)))
        .and_then(move |db_response| {
            let payments = db_response?;
            Ok(payments)
        })
        .and_then(move |payments| {
            let mut futures = vec![];
            debug!("Found {} pending payments", payments.len());
            for payment in payments {
                if payment.is_expired() {
                    debug!("payment {} expired: try to reject it", payment.id);
                    futures.push(Either::A(
                        fsm.send(RejectPayment {
                            payment: payment.clone(),
                        })
                        .map_err(|e| Error::General(s!(e)))
                        .and_then(|db_response| {
                            db_response?;
                            Ok(())
                        })
                        .or_else(move |e| {
                            error!("Cannot reject payment {}: {}", payment.id, e);
                            Ok(())
                        }),
                    ));
                    continue;
                }
                let res = wallet
                    .get_tx(&payment.wallet_tx_slate_id.clone().unwrap()) //we should have this field up to this moment
                    .and_then({
                        let fsm = fsm.clone();
                        let payment = payment.clone();
                        move |wallet_tx| {
                            if wallet_tx.confirmed {
                                info!("payment {} confirmed", payment.id);
                                let res = fsm
                                    .send(ConfirmPayment { payment, wallet_tx })
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
                        let payment_id = payment.id.clone();
                        move |e| {
                            warn!("Couldn't confirm payment {}: {}", payment_id, e);
                            Ok(())
                        }
                    });
                futures.push(Either::B(res));
            }
            join_all(futures).map(|_| ())
        });
    actix::spawn(res.map_err(|e| error!("Got an error in processing penging payments {}", e)));
}

fn process_unreported_payments(cron: &mut Cron, _: &mut Context<Cron>) {
    let res = cron
        .fsm
        .send(GetUnreportedPayments)
        .map_err(|e| Error::General(s!(e)))
        .and_then(move |db_response| {
            let payments = db_response?;
            Ok(payments)
        })
        .and_then({
            let fsm = cron.fsm.clone();
            move |payments| {
                let mut futures = vec![];
                debug!("Found {} unreported payments", payments.len());
                for payment in payments {
                    let payment_id = payment.id.clone();
                    futures.push(
                        fsm.send(ReportPayment { payment })
                            .map_err(|e| Error::General(s!(e)))
                            .and_then(|db_response| {
                                db_response?;
                                Ok(())
                            })
                            .or_else({
                                move |e| {
                                    warn!("Couldn't report payment {}: {}", payment_id, e);
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

fn process_expired_new_payouts(cron: &mut Cron, _: &mut Context<Cron>) {
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

fn process_expired_initialized_payouts(cron: &mut Cron, _: &mut Context<Cron>) {
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

fn process_pending_payouts(cron: &mut Cron, _: &mut Context<Cron>) {
    debug!("run process_pending_payouts");
    let wallet = cron.wallet.clone();
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
                    futures.push(Either::A(
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
                    ));
                    continue;
                }
                let res = wallet
                    .get_tx(&payout.wallet_tx_slate_id.clone().unwrap()) //we should have this field up to this moment
                    .and_then({
                        let fsm = fsm.clone();
                        let pending_payout = payout.clone();
                        let payout_id = payout.id.clone();
                        move |wallet_tx| {
                            if wallet_tx.confirmed {
                                info!("payout {} confirmed", payout_id);
                                let res = fsm
                                    .send(ConfirmPayout {
                                        pending_payout,
                                        wallet_tx,
                                    })
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
                        let payout_id = payout.id.clone();
                        move |e| {
                            warn!("Couldn't confirm payout {}: {}", payout_id, e);
                            Ok(())
                        }
                    });
                futures.push(Either::B(res));
            }
            join_all(futures).map(|_| ())
        });
    actix::spawn(res.map_err(|e| error!("Got an error in processing penging payouts {}", e)));
}
