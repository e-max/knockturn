use crate::db::{get_current_height, DbExecutor, RejectExpiredPayments};
use crate::errors::Error;
use crate::fsm::{
    get_unreported_confirmed_payments, get_unreported_rejected_payments, Fsm, Payment,
    PendingPayment, RejectPayment, ReportPayment,
};
use crate::models::{Transaction, TransactionStatus};
use crate::node::Node;
use crate::rates::RatesFetcher;
use crate::Pool;
use actix::prelude::*;
use actix_web::web::block;
use diesel::pg::PgConnection;
use diesel::{self, prelude::*};
use futures::future::{join_all, Future, FutureExt, TryFutureExt};

use log::*;
use std::collections::HashMap;

const REQUEST_BLOCKS_FROM_NODE: i64 = 10;

#[derive(Clone)]
pub struct Cron {
    db: Addr<DbExecutor>,
    node: Node,
    fsm: Addr<Fsm>,
    pool: Pool,
}

impl Actor for Cron {
    type Context = Context<Self>;

    fn started(&mut self, ctx: &mut Self::Context) {
        info!("Starting cron process");
        let rates = RatesFetcher::new(self.pool.clone());
        ctx.run_interval(
            std::time::Duration::new(5, 0),
            move |_instance: &mut Cron, _ctx: &mut Context<Self>| {
                rates.fetch();
            },
        );
        ctx.run_interval(std::time::Duration::new(5, 0), reject_expired_payments);
        ctx.run_interval(std::time::Duration::new(5, 0), process_pending_payments);

        ctx.run_interval(
            std::time::Duration::new(5, 0),
            process_unreported_confirmed_payments,
        );
        ctx.run_interval(
            std::time::Duration::new(5, 0),
            process_unreported_rejected_payments,
        );
        ctx.run_interval(std::time::Duration::new(5, 0), sync_with_node);
        ctx.run_interval(std::time::Duration::new(5, 0), autoconfirmation);
    }

    fn stopping(&mut self, _ctx: &mut Self::Context) -> Running {
        info!("Stop cron process");
        Running::Stop
    }
}

impl Cron {
    pub fn new(db: Addr<DbExecutor>, fsm: Addr<Fsm>, node: Node, pool: Pool) -> Self {
        Cron {
            db,
            fsm,
            node,
            pool,
        }
    }
    async fn process_pending_payments(&self) -> Result<(), Error> {
        debug!("run process_pending_payments");
        let payments: Vec<PendingPayment> = Payment::list(self.pool).await?;
        for payment in payments {
            if payment.is_expired() {
                debug!("payment {} expired: try to reject it", payment.id);
                let res: Result<(), Error> = self
                    .fsm
                    .send(RejectPayment {
                        payment: payment.clone(),
                    })
                    .await
                    .map_err(|e| Error::General(s!(e)))
                    .and_then(|db_response| {
                        db_response?;
                        Ok(())
                    })
                    .or_else(move |e| {
                        error!("Cannot reject payment {}: {}", payment.id, e);
                        Ok(())
                    });
                res?;
            }
        }
        Ok(())
    }

    async fn process_unreported_confirmed_payments(&self) -> Result<(), Error> {
        let payments = get_unreported_confirmed_payments(self.pool.clone()).await?;
        for payment in payments {
            let res: Result<(), Error> = self
                .fsm
                .send(ReportPayment { payment })
                .await
                .map_err(|e| Error::General(s!(e)))
                .and_then(|db_response| {
                    db_response?;
                    Ok(())
                })
                .or_else({
                    move |e| {
                        warn!("Couldn't report payment {}: {}", payment.id, e);
                        Ok(())
                    }
                });
            res?;
        }
        Ok(())
    }

    async fn process_unreported_rejected_payments(&self) -> Result<(), Error> {
        let payments = get_unreported_rejected_payments(self.pool.clone()).await?;
        for payment in payments {
            let res: Result<(), Error> = self
                .fsm
                .send(ReportPayment { payment })
                .await
                .map_err(|e| Error::General(s!(e)))
                .and_then(|db_response| {
                    db_response?;
                    Ok(())
                })
                .or_else({
                    move |e| {
                        warn!("Couldn't report payment {}: {}", payment.id, e);
                        Ok(())
                    }
                });
            res?;
        }
        Ok(())
    }
    async fn sync_with_node(&self) -> Result<(), Error> {
        debug!("run sync_with_node");

        let last_height = block::<_, _, Error>({
            let pool = self.pool.clone();
            move || {
                let conn: &PgConnection = &pool.get().unwrap();
                let last_height = get_current_height(conn)?;
                Ok(last_height)
            }
        })
        .await?;

        let blocks = self
            .node
            .blocks(last_height + 1, last_height + 1 + REQUEST_BLOCKS_FROM_NODE)
            .await?;
        let new_height = blocks
            .iter()
            .fold(last_height as u64, |current_height, block| {
                if block.header.height > current_height {
                    block.header.height
                } else {
                    current_height
                }
            });
        let commits: HashMap<String, i64> = blocks
            .iter()
            .flat_map(|block| block.outputs.iter())
            .filter(|o| !o.is_coinbase())
            .filter(|o| o.block_height.is_some())
            .map(|o| (o.commit.clone(), o.block_height.unwrap() as i64))
            .collect();
        debug!("Found {} non coinbase outputs", commits.len());
        block({
            let pool = self.pool.clone();
            move || {
                use crate::schema::transactions::dsl::*;
                let conn: &PgConnection = &pool.get().unwrap();
                conn.transaction(move || {
                    let txs = transactions
                        .filter(commit.eq_any(commits.keys()))
                        .load::<Transaction>(conn)?;

                    if txs.len() > 0 {
                        debug!("Found {} transactions which got into chain", txs.len());
                    }
                    for tx in txs {
                        let query = diesel::update(transactions.filter(id.eq(tx.id.clone())));

                        match tx.status {
                            TransactionStatus::Pending => query.set((
                                status.eq(TransactionStatus::InChain),
                                height.eq(commits.get(&tx.commit.unwrap()).unwrap()),
                            )),
                            TransactionStatus::Rejected => query.set((
                                status.eq(TransactionStatus::Refund),
                                height.eq(commits.get(&tx.commit.unwrap()).unwrap()),
                            )),
                            _ => {
                                return Err(Error::General(format!(
                                    "Transaction {} in chain although it has status {}",
                                    tx.id.clone(),
                                    tx.status
                                )))
                            }
                        }
                        .get_result(conn)
                        .map(|_: Transaction| ())
                        .map_err::<Error, _>(|e| e.into())?;
                    }
                    {
                        debug!("Set new last_height = {}", new_height);
                        use crate::schema::current_height::dsl::*;
                        diesel::update(current_height)
                            .set(height.eq(new_height as i64))
                            .execute(conn)
                            .map(|_| ())
                            .map_err::<Error, _>(|e| e.into())?;
                    }
                    Ok(())
                })
            }
        })
        .await?;

        Ok(())
    }

    async fn autoconfirmation(&self) -> Result<(), Error> {
        debug!("run autoconfirmation");
        block::<_, _, Error>({
            let pool = self.pool.clone();
            move || {
                let conn: &PgConnection = &pool.get().unwrap();
                let last_height = {
                    use crate::schema::current_height::dsl::*;
                    let last_height: i64 = current_height.select(height).first(conn)?;
                    last_height
                };

                use diesel::sql_query;
                sql_query(format!(
                    "UPDATE transactions SET status = 'confirmed' WHERE
            status = 'in_chain' and confirmations < {} - height",
                    last_height
                ))
                .execute(conn)?;
                Ok(())
            }
        })
        .await?;
        Ok(())
    }
}

fn reject_expired_payments(cron: &mut Cron, _: &mut Context<Cron>) {
    debug!("run process_expired_payments");

    let db = cron.db.clone();

    let fut = async move {
        db.send(RejectExpiredPayments)
            .await
            .map_err(|e| Error::from(e))
            .and_then(|db_response| {
                db_response?;
                Ok(())
            })
            .map_err(|e| error!("Got an error in rejecting exprired payments {}", e));
    };

    actix::spawn(fut);
}

fn process_pending_payments(cron: &mut Cron, _: &mut Context<Cron>) {
    let cron = cron.clone();
    actix::spawn(cron.process_pending_payments().map(|_| ()));
}

fn process_unreported_confirmed_payments(cron: &mut Cron, _: &mut Context<Cron>) {
    let cron = cron.clone();
    actix::spawn(cron.process_unreported_confirmed_payments().map(|_| ()));
}

fn process_unreported_rejected_payments(cron: &mut Cron, _: &mut Context<Cron>) {
    let cron = cron.clone();
    actix::spawn(cron.process_unreported_rejected_payments().map(|_| ()));
}

fn sync_with_node(cron: &mut Cron, _: &mut Context<Cron>) {
    let cron = cron.clone();
    actix::spawn(cron.sync_with_node().map(|_| ()));
}

fn autoconfirmation(cron: &mut Cron, _: &mut Context<Cron>) {
    let cron = cron.clone();
    actix::spawn(cron.autoconfirmation().map(|_| ()));
}
