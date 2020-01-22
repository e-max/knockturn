use crate::db::register_rate;
use crate::errors::Error;
use crate::Pool;
use actix_web::client::Client;
use actix_web::web::block;
use diesel::pg::PgConnection;
use log::*;
use serde::Deserialize;
use serde_json;
use std::collections::HashMap;
use std::str;

#[derive(Debug, Deserialize, Clone)]
pub struct Rates {
    grin: HashMap<String, f64>,
}

pub struct RatesFetcher {
    pool: Pool,
}

impl RatesFetcher {
    pub fn new(pool: Pool) -> Self {
        RatesFetcher { pool }
    }

    pub async fn fetch(&self) -> Result<Rates, Error> {
        let pool = self.pool.clone();
        let mut response = Client::default().get(
            "https://api.coingecko.com/api/v3/simple/price?ids=grin&vs_currencies=btc%2Cusd%2Ceur",
        )
        .header("Accept", "application/json")
        .send()
        .await
        .map_err(|e| {
            Error::General(format!("failed to fetch exchange rates: {:?}", e))
        })?;

        let body = response
            .body()
            .await
            .map_err(|e| Error::General(format!("failed to fetch exchange rates: {:?}", e)))?;

        let rates = serde_json::from_str::<Rates>(str::from_utf8(&body)?)
            .map_err(|e| Error::General(format!("failed to parse json: {:?}", e)))?;

        block::<_, _, Error>({
            let rates = rates.clone();
            move || {
                let conn: &PgConnection = &pool.get().unwrap();
                register_rate(rates.grin, conn)
            }
        })
        .await
        .map_err(|e| Error::General(format!("Failed to store rates in DB: {}", e)))?;

        Ok(rates)
    }
}
