use crate::errors::*;
use crate::schema::{merchants, orders, rates};
use chrono::{Duration, NaiveDateTime, Utc};
use diesel::backend::Backend;
use diesel::deserialize::{self, FromSql};
use diesel::pg::Pg;
use diesel::serialize::{self, Output, ToSql};
use diesel::sql_types::{Jsonb, SmallInt};
use serde::{Deserialize, Serialize};
use std::fmt;
use std::io;
use strum_macros::{Display, EnumString};
use uuid::Uuid;

const TTL_SECONDS: i64 = 20 * 60; //20 minutes

#[derive(Debug, Serialize, Deserialize, Queryable, Insertable, Identifiable)]
#[table_name = "merchants"]
pub struct Merchant {
    pub id: String,
    pub email: String,
    pub password: String,
    pub wallet_url: Option<String>,
    pub balance: i64,
    pub created_at: NaiveDateTime,
    pub token: String,
    pub callback_url: Option<String>,
    #[serde(skip_serializing)]
    pub token_2fa: Option<String>,
    #[serde(skip_serializing)]
    pub confirmed_2fa: bool,
}

/*
 * The status changes flow is as follows:
 * Unpaid - order was created but no attempts were maid to pay
 * Pending - user sent a slate and we succesfully sent it to wallet
 * Finalized - transaction was accepted to chain (Not used yet)
 * Confirmed - we got required number of confirmation for this transaction
 * Reported - we've reported result to merchant
 * Rejected - order spent too much time in Unpaid or Pending state
 * Dead - We mark Rejected order as Dead as soon as we report it to merchant
 */

#[derive(
    Clone, EnumString, Display, Debug, PartialEq, AsExpression, Serialize, Deserialize, FromSqlRow,
)]
#[sql_type = "SmallInt"]
pub enum OrderStatus {
    Unpaid,
    Pending,
    Rejected,
    Finalized,
    Confirmed,
}

impl<DB: Backend> ToSql<SmallInt, DB> for OrderStatus
where
    i16: ToSql<SmallInt, DB>,
{
    fn to_sql<W>(&self, out: &mut Output<W, DB>) -> serialize::Result
    where
        W: io::Write,
    {
        let v = match *self {
            OrderStatus::Unpaid => 1,
            OrderStatus::Pending => 2,
            OrderStatus::Rejected => 3,
            OrderStatus::Finalized => 4,
            OrderStatus::Confirmed => 5,
        };
        v.to_sql(out)
    }
}

impl<DB: Backend> FromSql<SmallInt, DB> for OrderStatus
where
    i16: FromSql<SmallInt, DB>,
{
    fn from_sql(bytes: Option<&DB::RawValue>) -> deserialize::Result<Self> {
        let v = i16::from_sql(bytes)?;
        Ok(match v {
            1 => OrderStatus::Unpaid,
            2 => OrderStatus::Pending,
            3 => OrderStatus::Rejected,
            4 => OrderStatus::Finalized,
            5 => OrderStatus::Confirmed,
            _ => return Err("replace me with a real error".into()),
        })
    }
}

#[derive(
    Debug, Serialize, Deserialize, Queryable, Insertable, Identifiable, Clone, AsExpression,
)]
#[table_name = "orders"]
pub struct Order {
    pub id: Uuid,
    pub external_id: String,
    pub merchant_id: String,
    pub grin_amount: i64,
    pub amount: Money,
    pub status: OrderStatus,
    pub confirmations: i32,
    pub email: Option<String>,
    pub created_at: NaiveDateTime,
    pub updated_at: NaiveDateTime,
    #[serde(skip_serializing)]
    pub reported: bool,
    #[serde(skip_serializing)]
    pub report_attempts: i32,
    #[serde(skip_serializing)]
    pub next_report_attempt: Option<NaiveDateTime>,
    #[serde(skip_serializing)]
    pub wallet_tx_id: Option<i64>,
    #[serde(skip_serializing)]
    pub wallet_tx_slate_id: Option<String>,
    pub message: String,
    pub slate_messages: Option<Vec<String>>,
}

impl Order {
    pub fn is_expired(&self) -> bool {
        self.created_at + Duration::seconds(TTL_SECONDS) < Utc::now().naive_utc()
    }

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
