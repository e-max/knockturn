use crate::models::Money;
use askama::Error;
use chrono::NaiveDateTime;
pub fn grin(nanogrins: &i64) -> Result<String, Error> {
    Ok(Money::from_grin(*nanogrins).to_string())
}

pub fn pretty_date(date: &NaiveDateTime) -> Result<String, Error> {
    Ok(date.format("%d.%m.%Y %H:%M:%S").to_string())
}
