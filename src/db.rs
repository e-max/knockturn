use crate::errors::*;
use crate::models::{Merchant, Order, OrderStatus};
use actix::{Actor, SyncContext};
use actix::{Handler, Message};
use chrono::{Duration, Local};
use diesel::pg::PgConnection;
use diesel::r2d2::{ConnectionManager, Pool};
use diesel::{self, prelude::*};
use serde::Deserialize;

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
pub struct GetMerchantById {
    pub id: String,
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

impl Message for CreateMerchant {
    type Result = Result<Merchant, Error>;
}

impl Message for GetMerchantById {
    type Result = Result<Option<Merchant>, Error>;
}

impl Message for CreateOrder {
    type Result = Result<Order, Error>;
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

impl Handler<GetMerchantById> for DbExecutor {
    type Result = Result<Option<Merchant>, Error>;

    fn handle(&mut self, msg: GetMerchantById, _: &mut Self::Context) -> Self::Result {
        use crate::schema::merchants::dsl::*;
        let conn: &PgConnection = &self.0.get().unwrap();
        merchants
            .find(msg.id)
            .get_result(conn)
            .optional()
            .map_err(|e| e.into())
    }
}

impl Handler<CreateOrder> for DbExecutor {
    type Result = Result<Order, Error>;

    fn handle(&mut self, msg: CreateOrder, _: &mut Self::Context) -> Self::Result {
        use crate::schema::merchants::dsl::*;
        use crate::schema::orders::dsl::*;
        let conn: &PgConnection = &self.0.get().unwrap();

        if !merchants
            .find(msg.merchant_id.clone())
            .get_result::<Merchant>(conn)
            .is_ok()
        {
            println!("merchant {} not found", msg.merchant_id);
            return Err(Error::InvalidEntity("merchant".to_owned()));
        }

        let new_order = Order {
            order_id: msg.order_id,
            merchant_id: msg.merchant_id,
            email: msg.email,
            callback_url: msg.callback_url,
            fiat_amount: msg.amount,
            currency: msg.currency,
            amount: msg.amount,
            status: OrderStatus::Unpaid as i32,
            confirmations: msg.confirmations,
            created_at: Local::now().naive_local() + Duration::hours(24),
            updated_at: Local::now().naive_local() + Duration::hours(24),
        };

        diesel::insert_into(orders)
            .values(&new_order)
            .get_result(conn)
            .map_err(|e| e.into())
    }
}
