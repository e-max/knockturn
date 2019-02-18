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

impl ToString for OrderStatus {
    fn to_string(&self) -> String {
        match self {
            OrderStatus::Unpaid => "Unpaid".to_owned(),
            OrderStatus::Received => "Received".to_owned(),
            OrderStatus::Rejected => "Rejected".to_owned(),
            OrderStatus::Finalized => "Finalized".to_owned(),
            OrderStatus::Confirmed => "Confirmed".to_owned(),
        }
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

pub struct Money {}
