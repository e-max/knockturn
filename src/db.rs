use crate::errors::*;
use crate::models::{Merchant, Money, Order, OrderStatus, Rate};
use actix::{Actor, SyncContext};
use actix::{Handler, Message};
use chrono::{Duration, Local};
use diesel::pg::PgConnection;
use diesel::r2d2::{ConnectionManager, Pool};
use diesel::{self, prelude::*};
use serde::Deserialize;
use std::collections::HashMap;

pub struct DbExecutor(pub Pool<ConnectionManager<PgConnection>>);

impl Actor for DbExecutor {
    type Context = SyncContext<Self>;
}

#[derive(Debug, Deserialize)]
pub struct CreateMerchant {
    pub id: String,
    pub email: String,
    pub password: String,
    pub wallet_url: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct GetMerchant {
    pub id: String,
}

#[derive(Debug, Deserialize)]
pub struct GetOrder {
    pub merchant_id: String,
    pub order_id: String,
}

#[derive(Debug, Deserialize)]
pub struct CreateOrder {
    pub merchant_id: String,
    pub order_id: String,
    pub amount: i64,
    pub currency: String,
    pub confirmations: i32,
    pub callback_url: String,
    pub email: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct RegisterRate {
    pub rates: HashMap<String, f64>,
}

#[derive(Debug, Deserialize)]
pub struct ConvertCurrency {
    pub amount: Money,
    pub to: String,
}

impl Message for CreateMerchant {
    type Result = Result<Merchant, Error>;
}

impl Message for GetMerchant {
    type Result = Result<Merchant, Error>;
}

impl Message for GetOrder {
    type Result = Result<Order, Error>;
}

impl Message for CreateOrder {
    type Result = Result<Order, Error>;
}

impl Message for RegisterRate {
    type Result = Result<(), Error>;
}

impl Message for ConvertCurrency {
    type Result = Result<Money, Error>;
}

impl Handler<CreateMerchant> for DbExecutor {
    type Result = Result<Merchant, Error>;

    fn handle(&mut self, msg: CreateMerchant, _: &mut Self::Context) -> Self::Result {
        use crate::schema::merchants::dsl::*;
        let conn: &PgConnection = &self.0.get().unwrap();

        let new_merchant = Merchant {
            id: msg.id,
            email: msg.email,
            password: msg.password,
            wallet_url: msg.wallet_url,
            balance: 0,
            created_at: Local::now().naive_local() + Duration::hours(24),
        };

        diesel::insert_into(merchants)
            .values(&new_merchant)
            .get_result(conn)
            .map_err(|e| e.into())
    }
}

impl Handler<GetMerchant> for DbExecutor {
    type Result = Result<Merchant, Error>;

    fn handle(&mut self, msg: GetMerchant, _: &mut Self::Context) -> Self::Result {
        use crate::schema::merchants::dsl::*;
        let conn: &PgConnection = &self.0.get().unwrap();
        merchants
            .find(msg.id)
            .get_result(conn)
            .map_err(|e| e.into())
    }
}

impl Handler<GetOrder> for DbExecutor {
    type Result = Result<Order, Error>;

    fn handle(&mut self, msg: GetOrder, _: &mut Self::Context) -> Self::Result {
        use crate::schema::orders::dsl::*;
        let conn: &PgConnection = &self.0.get().unwrap();
        orders
            .find((msg.merchant_id, msg.order_id))
            .get_result(conn)
            .map_err(|e| e.into())
    }
}

impl Handler<CreateOrder> for DbExecutor {
    type Result = Result<Order, Error>;

    fn handle(&mut self, msg: CreateOrder, _: &mut Self::Context) -> Self::Result {
        use crate::schema::merchants::dsl::*;
        use crate::schema::orders::dsl::*;
        use crate::schema::rates::dsl::*;

        let conn: &PgConnection = &self.0.get().unwrap();

        if !merchants
            .find(msg.merchant_id.clone())
            .get_result::<Merchant>(conn)
            .is_ok()
        {
            return Err(Error::InvalidEntity("merchant".to_owned()));
        }

        let exch_rate = match rates
            .find(&msg.currency)
            .get_result::<Rate>(conn)
            .optional()?
        {
            None => return Err(Error::UnsupportedCurrency(msg.currency)),
            Some(v) => v,
        };

        //let precision: f64 = if msg.currency == "btc " {
        //    100_000_000.0
        //} else {
        //    100.0
        //};

        //let conv_rate = (precision * exch_rate.rate) as i64;
        let fiat = Money {
            amount: msg.amount,
            currency: msg.currency,
        };
        let grins = fiat.convert_to("grin", exch_rate.rate);

        let new_order = Order {
            order_id: msg.order_id,
            merchant_id: msg.merchant_id,
            email: msg.email,
            callback_url: msg.callback_url,
            currency: fiat.currency,
            amount: fiat.amount,
            grin_amount: grins.amount, //msg.amount * 1_000_000_000 / conv_rate,
            status: OrderStatus::Unpaid as i32,
            confirmations: msg.confirmations,
            created_at: Local::now().naive_local(),
            updated_at: Local::now().naive_local(),
        };

        diesel::insert_into(orders)
            .values(&new_order)
            .get_result(conn)
            .map_err(|e| e.into())
    }
}

impl Handler<RegisterRate> for DbExecutor {
    type Result = Result<(), Error>;

    fn handle(&mut self, msg: RegisterRate, _: &mut Self::Context) -> Self::Result {
        use crate::schema::rates::dsl::*;
        let conn: &PgConnection = &self.0.get().unwrap();

        for (currency, new_rate) in msg.rates {
            let new_rate = Rate {
                id: currency,
                rate: new_rate,
                updated_at: Local::now().naive_local(),
            };

            diesel::insert_into(rates)
                .values(&new_rate)
                .on_conflict(id)
                .do_update()
                .set(&new_rate)
                .get_result::<Rate>(conn)
                .map_err(|e| Error::from(e))?;
        }
        Ok(())
    }
}
