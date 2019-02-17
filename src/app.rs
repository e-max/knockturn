use crate::db::DbExecutor;
use crate::handlers::*;
use actix::prelude::*;
use actix_web::{http::Method, middleware, App};

pub struct AppState {
    pub db: Addr<DbExecutor>,
}

pub fn create_app(db: Addr<DbExecutor>) -> App<AppState> {
    App::with_state(AppState { db })
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
}
