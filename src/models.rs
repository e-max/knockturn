use crate::schema::{merchants, orders, rates, txs};
use chrono::NaiveDateTime;
use serde::{Deserialize, Serialize};
use std::fmt;

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
    pub grin_amount: i64,
    pub currency: String,
    pub amount: i64,
    pub status: i32,
    pub confirmations: i32,
    pub callback_url: String,
    pub email: Option<String>,
    pub created_at: NaiveDateTime,
    pub updated_at: NaiveDateTime,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Money {
    pub amount: i64,
    pub currency: String,
}

impl Money {
    pub fn new(amount: i64, currency: String) -> Self {
        Money {
            amount,
            currency: currency.to_lowercase(),
        }
    }

    pub fn convert_to(&self, currency: &str, rate: f64) -> Money {
        let amount = self.amount * Money::currency_precision(currency)
            / (self.precision() as f64 * rate) as i64;
        Money {
            amount,
            currency: currency.to_owned(),
        }
    }

    pub fn precision(&self) -> i64 {
        Money::currency_precision(&self.currency)
    }

    pub fn currency_precision(currency: &str) -> i64 {
        match currency {
            "btc" => 100_000_000,
            "grin" => 1_000_000_000,
            _ => 100,
        }
    }
}

impl fmt::Display for Money {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        let pr = self.precision();
        let grins = self.amount / pr;
        let mgrins = self.amount % pr;
        match self.currency.as_str() {
            "btc" => write!(f, "{}.{:08} {}", grins, mgrins, self.currency),
            "grin" => write!(f, "{}.{:09} {}", grins, mgrins, self.currency),
            _ => write!(f, "{}.{:02} {}", grins, mgrins, self.currency),
        }
    }
}

#[derive(Debug, Serialize, Deserialize, Queryable, Insertable, Identifiable, AsChangeset)]
#[table_name = "rates"]
pub struct Rate {
    pub id: String,
    pub rate: f64,
    pub updated_at: NaiveDateTime,
}

#[derive(Debug, Serialize, Deserialize, Queryable, Insertable, Identifiable)]
#[table_name = "txs"]
#[primary_key(slate_id)]
pub struct Tx {
    pub slate_id: String,
    pub created_at: NaiveDateTime,
    pub confirmed: bool,
    pub confirmed_at: Option<NaiveDateTime>,
    pub fee: Option<i64>,
    pub messages: Vec<String>,
    pub num_inputs: i64,
    pub num_outputs: i64,
    pub tx_type: String,
    pub order_id: String,
    pub merchant_id: String,
    pub updated_at: NaiveDateTime,
}
