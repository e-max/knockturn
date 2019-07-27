#[macro_use]
mod macros;

pub mod app;
pub mod cron;
pub mod cron_payout;
pub mod db;
pub mod errors;
pub mod extractor;
pub mod filters;
pub mod fsm;
pub mod fsm_payout;
pub mod handlers;
pub mod models;
pub mod node;
pub mod qrcode;
pub mod rates;
#[allow(unused_imports)]
pub mod schema;
mod ser;
#[cfg(test)]
pub mod test_utils;
pub mod totp;
pub mod wallet;

#[macro_use]
extern crate diesel;

use diesel::pg::PgConnection;
use diesel::r2d2;
pub use errors::Error;

pub type Pool = r2d2::Pool<r2d2::ConnectionManager<PgConnection>>;
