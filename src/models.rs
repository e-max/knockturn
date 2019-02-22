use crate::errors::*;
use crate::schema::{merchants, orders, rates, txs};
use chrono::NaiveDateTime;
use diesel::deserialize::{self, FromSql};
use diesel::pg::Pg;
use diesel::serialize::{self, Output, ToSql};
use diesel::sql_types::Jsonb;
use serde::{Deserialize, Serialize};
use std::fmt;
use uuid::Uuid;

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
pub struct Order {
    pub id: Uuid,
    pub external_id: String,
    pub merchant_id: String,
    pub grin_amount: i64,
    pub amount: Money,
    pub status: i32,
    pub confirmations: i32,
    pub email: Option<String>,
    pub created_at: NaiveDateTime,
    pub updated_at: NaiveDateTime,
}

impl Order {
    pub fn grins(&self) -> Money {
        Money::new(self.grin_amount, Currency::GRIN)
    }
}

#[derive(Debug, Serialize, Deserialize, Clone, Copy)]
pub enum Currency {
    GRIN = 0,
    BTC = 1,
    EUR = 2,
    USD = 3,
}

impl Currency {
    pub fn precision(&self) -> i64 {
        match self {
            Currency::BTC => 100_000_000,
            Currency::GRIN => 1_000_000_000,
            Currency::EUR | Currency::USD => 100,
        }
    }
}

impl fmt::Display for Currency {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        let s = match self {
            Currency::BTC => "BTC",
            Currency::GRIN => "GRIN",
            Currency::EUR => "EUR",
            Currency::USD => "USD",
        };
        write!(f, "{}", s)
    }
}

impl std::str::FromStr for Currency {
    type Err = Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_uppercase().as_str() {
            "GRIN" => Ok(Currency::GRIN),
            "BTC" => Ok(Currency::BTC),
            "USD" => Ok(Currency::USD),
            "EUR" => Ok(Currency::EUR),
            _ => Err(Error::UnsupportedCurrency(s!(s))),
        }
    }
}

#[derive(Debug, Serialize, Deserialize, AsExpression, FromSqlRow, Clone, Copy)]
#[sql_type = "Jsonb"]
pub struct Money {
    pub amount: i64,
    pub currency: Currency,
}

impl Money {
    pub fn new(amount: i64, currency: Currency) -> Self {
        Money { amount, currency }
    }

    pub fn convert_to(&self, currency: Currency, rate: f64) -> Money {
        let amount =
            self.amount * currency.precision() / (self.currency.precision() as f64 * rate) as i64;
        Money {
            amount,
            currency: currency,
        }
    }
}

impl ToSql<Jsonb, Pg> for Money {
    fn to_sql<W: std::io::Write>(&self, out: &mut Output<W, Pg>) -> serialize::Result {
        out.write_all(&[1])?;
        serde_json::to_writer(out, self)
            .map(|_| serialize::IsNull::No)
            .map_err(Into::into)
    }
}

impl FromSql<Jsonb, Pg> for Money {
    fn from_sql(bytes: Option<&[u8]>) -> deserialize::Result<Self> {
        let bytes = not_none!(bytes);
        if bytes[0] != 1 {
            return Err("Unsupported JSONB encoding version".into());
        }
        serde_json::from_slice(&bytes[1..]).map_err(Into::into)
    }
}

impl fmt::Display for Money {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        let pr = self.currency.precision();
        let grins = self.amount / pr;
        let mgrins = self.amount % pr;
        match self.currency {
            Currency::BTC => write!(f, "{}.{:08} {}", grins, mgrins, self.currency),
            Currency::GRIN => write!(f, "{}.{:09} {}", grins, mgrins, self.currency),
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
    pub order_id: Uuid,
    pub updated_at: NaiveDateTime,
}
