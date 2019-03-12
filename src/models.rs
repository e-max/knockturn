use crate::errors::*;
use crate::schema::{merchants, rates, transactions};
use chrono::{Duration, NaiveDateTime, Utc};
use diesel::backend::Backend;
use diesel::deserialize::{self, FromSql};
use diesel::pg::Pg;
use diesel::serialize::{self, Output, ToSql};
use diesel::sql_types::{Jsonb, SmallInt};
use diesel_derive_enum::DbEnum;
use serde::{Deserialize, Serialize};
use std::fmt;
use std::io;
use strum_macros::{Display, EnumString};
use uuid::Uuid;

pub const NEW_PAYMENT_TTL_SECONDS: i64 = 15 * 60; //15 minutes since creation time
pub const PENDING_PAYMENT_TTL_SECONDS: i64 = 7 * 60; //7  minutes since became pending

pub const NEW_PAYOUT_TTL_SECONDS: i64 = 5 * 60; //5  minutes since creation time
pub const INITIALIZED_PAYOUT_TTL_SECONDS: i64 = 5 * 60; //5  minutes since creation time
pub const PENDING_PAYOUT_TTL_SECONDS: i64 = 15 * 60; //15 minutes since became pending

#[derive(Debug, Serialize, Deserialize, Queryable, Insertable, Identifiable, Clone)]
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
 * The status of payment changes flow is as follows:
 * New - transaction was created but no attempts were maid to pay
 * Pending - user sent a slate and we succesfully sent it to wallet
 * Finalized - transaction was accepted to chain (Not used yet)
 * Confirmed - we got required number of confirmation for this transaction
 * Rejected - transaction spent too much time in New or Pending state
 *
 * The status of payout changes as follows:
 * New - payout created in db
 * Initialized - we created transaction in wallet, created slate and sent it to merchant
 * Pending - user returned to us slate, we finalized it in wallet and wait for required number of confimations
 * Confirmed - we got required number of confimations
 */

#[derive(Debug, PartialEq, DbEnum, Serialize, Deserialize, Clone, Copy, EnumString, Display)]
pub enum TransactionStatus {
    New,
    Pending,
    Rejected,
    Finalized,
    Confirmed,
    Initialized,
}

#[derive(Debug, PartialEq, DbEnum, Serialize, Deserialize, Clone, Copy, EnumString, Display)]
pub enum TransactionType {
    Payment,
    Payout,
}

#[derive(
    Debug, Serialize, Deserialize, Queryable, Insertable, Identifiable, Clone, AsExpression,
)]
#[table_name = "transactions"]
pub struct Transaction {
    pub id: Uuid,
    pub external_id: String,
    pub merchant_id: String,
    pub grin_amount: i64,
    pub amount: Money,
    pub status: TransactionStatus,
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
    pub knockturn_fee: Option<i64>,
    pub transfer_fee: Option<i64>,
    #[serde(skip_serializing)]
    pub real_transfer_fee: Option<i64>,
    pub transaction_type: TransactionType,
}

impl Transaction {
    pub fn is_expired(&self) -> bool {
        match self.time_until_expired() {
            Some(time) => time < Duration::zero(),
            None => false,
        }
    }

    pub fn time_until_expired(&self) -> Option<Duration> {
        let expiration_time = match (self.transaction_type, self.status) {
            (TransactionType::Payment, TransactionStatus::New) => {
                Some(self.created_at + Duration::seconds(NEW_PAYMENT_TTL_SECONDS))
            }
            (TransactionType::Payment, TransactionStatus::Pending) => {
                Some(self.updated_at + Duration::seconds(PENDING_PAYMENT_TTL_SECONDS))
            }
            (TransactionType::Payout, TransactionStatus::New) => {
                Some(self.created_at + Duration::seconds(NEW_PAYOUT_TTL_SECONDS))
            }
            (TransactionType::Payout, TransactionStatus::Initialized) => {
                Some(self.created_at + Duration::seconds(PENDING_PAYOUT_TTL_SECONDS))
            }
            (TransactionType::Payout, TransactionStatus::Pending) => {
                Some(self.updated_at + Duration::seconds(PENDING_PAYOUT_TTL_SECONDS))
            }
            (_, _) => None,
        };
        expiration_time.map(|exp_time| exp_time - Utc::now().naive_utc())
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

impl From<i64> for Money {
    fn from(val: i64) -> Money {
        Money::from_grin(val)
    }
}

impl Money {
    pub fn new(amount: i64, currency: Currency) -> Self {
        Money { amount, currency }
    }

    pub fn from_grin(amount: i64) -> Self {
        Money {
            amount: amount,
            currency: Currency::GRIN,
        }
    }

    pub fn convert_to(&self, currency: Currency, rate: f64) -> Money {
        let amount =
            self.amount * currency.precision() / (self.currency.precision() as f64 * rate) as i64;
        Money {
            amount,
            currency: currency,
        }
    }

    pub fn amount(&self) -> String {
        let pr = self.currency.precision();
        let grins = self.amount / pr;
        let mgrins = self.amount % pr;
        match self.currency {
            Currency::BTC => format!("{}.{:08}", grins, mgrins),
            Currency::GRIN => format!("{}.{:09}", grins, mgrins),
            _ => format!("{}.{:02}", grins, mgrins),
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
        write!(f, "{} {}", self.amount(), self.currency)
    }
}

#[derive(Debug, Serialize, Deserialize, Queryable, Insertable, Identifiable, AsChangeset)]
#[table_name = "rates"]
pub struct Rate {
    pub id: String,
    pub rate: f64,
    pub updated_at: NaiveDateTime,
}
