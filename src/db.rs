use crate::models::Merchant;
use actix::{Actor, SyncContext};
use actix::{Handler, Message};
use chrono::{Duration, Local};
use diesel::pg::PgConnection;
use diesel::r2d2::{ConnectionManager, Pool};
use diesel::result::{DatabaseErrorKind, Error, Error::DatabaseError};
use diesel::{self, prelude::*};
use serde::Deserialize;
use uuid::Uuid;

pub struct DbExecutor(pub Pool<ConnectionManager<PgConnection>>);

impl Actor for DbExecutor {
    type Context = SyncContext<Self>;
}

#[derive(Debug, Deserialize)]
pub struct CreateMerchant {
    pub email: String,
    pub password: String,
    pub wallet_url: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct GetMerchantById {
    pub id: String,
}

impl Message for CreateMerchant {
    type Result = Result<Merchant, Error>;
}

impl Message for GetMerchantById {
    type Result = Result<Option<Merchant>, Error>;
}

impl Handler<CreateMerchant> for DbExecutor {
    type Result = Result<Merchant, Error>;

    fn handle(&mut self, msg: CreateMerchant, _: &mut Self::Context) -> Self::Result {
        use crate::schema::merchants::dsl::*;
        let conn: &PgConnection = &self.0.get().unwrap();

        let new_merchant = Merchant {
            id: Uuid::new_v4(),
            email: msg.email.clone(),
            password: msg.password.clone(),
            wallet_url: msg.wallet_url.clone(),
            balance: 0,
            created_at: Local::now().naive_local() + Duration::hours(24),
        };

        diesel::insert_into(merchants)
            .values(&new_merchant)
            .get_result(conn)
    }
}

impl Handler<GetMerchantById> for DbExecutor {
    type Result = Result<Option<Merchant>, Error>;

    fn handle(&mut self, msg: GetMerchantById, _: &mut Self::Context) -> Self::Result {
        use crate::schema::merchants::dsl::*;
        let conn: &PgConnection = &self.0.get().unwrap();
        let merchant_id = match Uuid::parse_str(&msg.id) {
            Ok(mid) => mid,
            Err(_) => return Ok(None),
        };
        merchants.find(merchant_id).get_result(conn).optional()
    }
}
