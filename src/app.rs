use crate::db::DbExecutor;
use crate::handlers::*;
use crate::wallet::Wallet;
use actix::prelude::*;
use actix_web::{http::Method, middleware, App};

pub struct AppState {
    pub db: Addr<DbExecutor>,
    pub wallet: Wallet,
}

pub fn create_app(db: Addr<DbExecutor>, wallet: Wallet) -> App<AppState> {
    App::with_state(AppState { db, wallet })
        .middleware(middleware::Logger::new("\"%r\" %s %b %Dms"))
        .resource("/merchants", |r| {
            r.method(Method::POST).with(create_merchant)
        })
        .resource("/merchants/{merchant_id}", |r| {
            r.method(Method::GET).with(get_merchant)
        })
        .resource("/merchants/{merchant_id}/orders", |r| {
            r.method(Method::POST).with(create_order)
        })
        .resource("/merchants/{merchant_id}/orders/{order_id}", |r| {
            r.method(Method::GET).with(get_order);
            r.method(Method::POST).with(pay_order);
        })
        .resource("/tx", |r| r.method(Method::GET).with(get_tx))
}
