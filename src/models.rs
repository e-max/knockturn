use crate::schema::{merchants, orders};
use chrono::NaiveDateTime;
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize, Queryable, Insertable, Identifiable)]
#[table_name = "merchants"]
pub struct Merchant {
    pub id: String,
    pub email: String,
    pub password: String,
    pub wallet_url: Option<String>,
    pub balance: i64,
    pub created_at: NaiveDateTime,
}

impl Merchant {
    // this is just a helper function to remove password from user just before we return the value out later
    pub fn remove_pwd(mut self) -> Self {
        self.password = "".to_string();
        self
    }
}

enum_from_primitive! {
#[derive(Debug, PartialEq)]
pub enum OrderStatus {
    Unpaid  = 0,
    Received = 1,
    Rejected = 2,
    Finalized = 3,
    Confirmed = 4,
}
}

#[derive(Debug, Serialize, Deserialize, Queryable, Insertable, Identifiable)]
#[table_name = "orders"]
#[primary_key(merchant_id, order_id)]
pub struct Order {
    pub order_id: String,
    pub merchant_id: String,
    pub fiat_amount: i64,
    pub currency: String,
    pub amount: i64,
    pub status: i32,
    pub confirmations: i32,
    pub callback_url: String,
    pub email: Option<String>,
    pub created_at: NaiveDateTime,
    pub updated_at: NaiveDateTime,
}
