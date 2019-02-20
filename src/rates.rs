use crate::db::{DbExecutor, RegisterRate};
use crate::errors::*;
use actix::prelude::*;
use actix_web::client;
use actix_web::HttpMessage;
use futures;
use futures::future::{err, ok, result, Future};
use log::{debug, error, info};
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
}

impl Actor for RatesFetcher {
    type Context = Context<Self>;
}

pub struct FetchRates {}

impl Message for FetchRates {
    type Result = Result<(), Error>;
}

impl Handler<FetchRates> for RatesFetcher {
    type Result = Result<(), Error>; // Box<Future<Item = (), Error = Error>>; // ;

    fn handle(&mut self, msg: FetchRates, _: &mut Self::Context) -> Self::Result {
        println!("fetching");
        Ok(())
        //client::get(
        //    "https://api.coingecko.com/api/v3/simple/price?ids=grin&vs_currencies=btc%2Cusd%2Ceur",
        //)
        //.header("User-Agent", "Actix-web")
        //.header("Accept", "application/json")
        //.finish()
        //.unwrap()
        //.send()
        //.map_err(|e| Error::Fetch(format!("failed to fetch exchange rates: {:?}", e)))
        //.and_then(|response| {
        //    response
        //        .body()
        //        .map_err(|e| Error::Fetch(format!("Payload error: {:?}", e)))
        //        .and_then(move |body| Ok(str::from_utf8(&body)?))
        //        //.from_err(Error::Fetch(format!("failed to parse body")))
        //        .and_then(|str| Ok(serde_json::from_str::<Rates>(&str)?))
        //        .map_err(|e| Error::Fetch(format!("failed to parse json: {:?}", e)))
        //        .and_then(|rates| {
        //            self.db
        //                .send(RegisterRate { rates: rates.grin })
        //                .map_err(|_| Error::Fetch(format!("failed to parse body")))
        //                .and_then(|db_response| db_response)
        //                .from_err()
        //                .and_then(|_| Ok(()))
        //        })
        //})
    }
}
impl RatesFetcher {
    pub fn new(db: Addr<DbExecutor>) -> Self {
        RatesFetcher { db }
    }

    pub fn fetch(&self) -> impl Future<Item = (), Error = ()> + '_ {
        let db = self.db.clone();
        client::get(
            "https://api.coingecko.com/api/v3/simple/price?ids=grin&vs_currencies=btc%2Cusd%2Ceur",
        )
        .header("User-Agent", "Actix-web")
        .header("Accept", "application/json")
        .finish()
        .unwrap()
        .send()
        .map_err(|e| Error::Fetch(format!("failed to fetch exchange rates: {:?}", e)))
        .and_then(|response| {
            response
                .body()
                .map_err(|e| Error::Fetch(format!("Payload error: {:?}", e)))
                .and_then(move |body| {
                    match str::from_utf8(&body) {
                        Ok(v) => ok(v.to_owned()),
                        Err(e) => err(Error::Fetch(format!("failed to parse body"))),
                    }
                    //result(
                    //    str::from_utf8(&body)
                    //        .map_err(|_| Error::Fetch(format!("failed to parse body"))),
                    //)
                })
                .and_then(|str| {
                    result(
                        serde_json::from_str::<Rates>(&str)
                            .map_err(|e| Error::Fetch(format!("failed to parse json: {:?}", e))),
                    )
                })
                .and_then(move |rates| {
                    db.send(RegisterRate { rates: rates.grin })
                        .map_err(|_| Error::Fetch(format!("failed to parse body")))
                        .and_then(|db_response| db_response)
                        .from_err()
                        .and_then(|_| Ok(()))
                })
        })
    }
}
