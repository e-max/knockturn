#[macro_use]
mod macros;

mod app;
mod cron;
mod db;
mod errors;
mod handlers;
mod models;
mod rates;
mod schema;
mod wallet;

#[macro_use]
extern crate diesel;
#[macro_use]
extern crate failure;
#[macro_use]
extern crate enum_primitive;

use actix::prelude::*;
use actix_web::server;
//use actix_web::{http, server, App, HttpRequest, Path};
use crate::db::DbExecutor;
use crate::wallet::Wallet;
use diesel::{r2d2::ConnectionManager, PgConnection};
use dotenv::dotenv;
use env_logger;
use log::info;
use std::env;

fn main() {
    env_logger::init();
    dotenv().ok();
    let database_url = env::var("DATABASE_URL").expect("DATABASE_URL must be set");
    let sys = actix::System::new("Knockout");
    std::env::set_var("RUST_LOG", "actix_web=info");

    let manager = ConnectionManager::<PgConnection>::new(database_url);
    let pool = r2d2::Pool::builder()
        .build(manager)
        .expect("Failed to create pool.");

    let address: Addr<DbExecutor> = SyncArbiter::start(10, move || DbExecutor(pool.clone()));

    let wallet_url = env::var("WALLET_URL").expect("WALLET_URL must be set");
    let wallet_user = env::var("WALLET_USER").expect("WALLET_USER must be set");
    let wallet_pass = env::var("WALLET_PASS").expect("WALLET_PASS must be set");

    let wallet = Wallet::new(&wallet_url, &wallet_user, &wallet_pass);

    info!("Starting");
    let cron_db = address.clone();
    let _cron = Arbiter::start(move |_| cron::Cron::new(cron_db));

    server::new(move || app::create_app(address.clone(), wallet.clone()))
        .bind("0.0.0.0:3000")
        .expect("Can not bind to '0.0.0.0:3000'")
        .start();

    sys.run();
}

//#[derive(Deserialize)]
//struct TxPath {
//    merchant_id: String,
//    order_id: String,
//}
//
//fn tx_home(tx_path: Path<TxPath>) -> String {
//    format!(
//        "Tx home\n merchant {}\n order_id {}\n",
//        tx_path.merchant_id, tx_path.order_id
//    )
//}
//
//fn main() {
//    server::new(|| {
//        App::new().resource("/orders/{merchant_id}/{order_id}", |r| {
//            r.method(http::Method::GET).with(tx_home)
//        })
//    })
//    .bind("127.0.0.1:8088")
//    .unwrap()
//    .run();
//}
