use crate::db::{register_rate, DbExecutor};
use crate::errors::Error;
use actix::prelude::*;
use actix_web::client::Client;
use actix_web::web::block;
use diesel::pg::PgConnection;
use diesel::r2d2::{ConnectionManager, Pool};
use futures;
use futures::future::{err, ok, result, Future};
use log::*;
use serde::Deserialize;
use serde_json;
use std::collections::HashMap;
use std::str;

#[derive(Debug, Deserialize)]
struct Rates {
    grin: HashMap<String, f64>,
}

pub struct RatesFetcher {
    db: Addr<DbExecutor>,
    pool: Pool<ConnectionManager<PgConnection>>,
}

impl RatesFetcher {
    pub fn new(db: Addr<DbExecutor>, pool: Pool<ConnectionManager<PgConnection>>) -> Self {
        RatesFetcher { db, pool }
    }

    pub fn fetch(&self) {
        let db = self.db.clone();
        let pool = self.pool.clone();
        let f = Client::default().get(
            "https://api.coingecko.com/api/v3/simple/price?ids=grin&vs_currencies=btc%2Cusd%2Ceur",
        )
        .header("Accept", "application/json")
        .send()
        .map_err(|e| {
            error!("failed to fetch exchange rates: {:?}", e);
            ()
        })
        .and_then(|mut response| {
            response
                .body()
                .map_err(|e| {
                    error!("Payload error: {:?}", e);
                    ()
                })
                .and_then(move |body| match str::from_utf8(&body) {
                    Ok(v) => ok(v.to_owned()),
                    Err(e) => {
                        error!("failed to parse body: {:?}", e);
                        err(())
                    }
                })
                .and_then(|str| {
                    result(serde_json::from_str::<Rates>(&str).map_err(|e| {
                        error!("failed to parse json: {:?}", e);
                        ()
                    }))
                })
                .and_then(move |rates| {
                    block::<_, _, Error>({
                        move || {
                            let conn: &PgConnection = &pool.get().unwrap();
                            register_rate(rates.grin, conn)
                        }}).map_err(|e| { 
                            error!("Failed to store rates in DB: {}", e);
                        })
                })
        });
        actix::spawn(f);
    }
}
