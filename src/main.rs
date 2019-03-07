#[macro_use]
mod macros;

mod app;
mod clients;
mod cron;
mod db;
mod errors;
mod fsm;
mod handlers;
mod middleware;
mod models;
mod rates;
mod schema;
mod totp;
mod wallet;

#[macro_use]
extern crate diesel;
#[macro_use]
extern crate failure;

use actix::prelude::*;
use actix_web::server;
//use actix_web::{http, server, App, HttpRequest, Path};
use crate::db::DbExecutor;
use crate::fsm::Fsm;
use crate::wallet::Wallet;
use diesel::{r2d2::ConnectionManager, PgConnection};
use dotenv::dotenv;
use env_logger;
use log::info;
use std::env;

fn main() {
    dotenv().ok();

    env_logger::init();

    let cookie_secret = env::var("COOKIE_SECRET").expect("COOKIE_SECRET must be set");
    let database_url = env::var("DATABASE_URL").expect("DATABASE_URL must be set");
    let host = env::var("HOST").unwrap_or("0.0.0.0:3000".to_owned());
    let _ = env::var("DOMAIN").expect("DOMAIN must be set");
    let sys = actix::System::new("Knockout");

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

    let fsm: Addr<Fsm> = Arbiter::start({
        let wallet = wallet.clone();
        let db = address.clone();
        move |_| Fsm {
            db: db,
            wallet: wallet,
        }
    });
    let _cron = Arbiter::start({
        let wallet = wallet.clone();
        let fsm = fsm.clone();
        move |_| cron::Cron::new(cron_db, wallet, fsm)
    });

    server::new(move || {
        app::create_app(
            address.clone(),
            wallet.clone(),
            fsm.clone(),
            cookie_secret.as_bytes(),
        )
    })
    .bind(&host)
    .expect(&format!("Can not bind to '{}'", &host))
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
