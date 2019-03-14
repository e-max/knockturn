use crate::models::Money;
use crate::models::{Transaction, TransactionStatus, TransactionType};
use askama::Error;
use chrono::{Duration, NaiveDateTime};
use chrono_humanize::{Accuracy, HumanTime, Tense};
pub fn grin(nanogrins: &i64) -> Result<String, Error> {
    Ok(Money::from_grin(*nanogrins).to_string())
}

pub fn pretty_date(date: &NaiveDateTime) -> Result<String, Error> {
    Ok(date.format("%d.%m.%Y %H:%M:%S").to_string())
}

pub fn transaction_color(tx: &Transaction) -> Result<&'static str, Error> {
    Ok(match (tx.transaction_type, tx.status) {
        (TransactionType::Payout, TransactionStatus::Confirmed) => "success",
        (TransactionType::Payout, TransactionStatus::Pending) => "info",
        (TransactionType::Payment, TransactionStatus::Rejected) => "secondary",
        (_, _) => "light",
    })
}

pub fn duration(duration: &Duration) -> Result<String, Error> {
    let ht = HumanTime::from(*duration);
    Ok(ht.to_text_en(Accuracy::Precise, Tense::Present))
}
