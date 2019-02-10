mod app;
mod db;
mod errors;
mod handlers;
mod models;
mod schema;

#[macro_use]
extern crate diesel;
#[macro_use]
extern crate failure;

use actix::prelude::*;
use actix_web::server;
//use actix_web::{http, server, App, HttpRequest, Path};
use crate::db::DbExecutor;
use diesel::prelude::*;
use diesel::{r2d2::ConnectionManager, PgConnection};
use dotenv::dotenv;
use serde::Deserialize;
use std::env;

fn main() {
    dotenv().ok();
    let database_url = env::var("DATABASE_URL").expect("DATABASE_URL must be set");
    let sys = actix::System::new("Knockout");

    let manager = ConnectionManager::<PgConnection>::new(database_url);
    let pool = r2d2::Pool::builder()
        .build(manager)
        .expect("Failed to create pool.");

    let address: Addr<DbExecutor> = SyncArbiter::start(10, move || DbExecutor(pool.clone()));

    server::new(move || app::create_app(address.clone()))
        .bind("127.0.0.1:3000")
        .expect("Can not bind to '127.0.0.1:3000'")
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
