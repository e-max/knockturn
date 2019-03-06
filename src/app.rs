use crate::db::DbExecutor;
use crate::fsm::Fsm;
use crate::handlers::*;
use crate::middleware::*;
use crate::wallet::Wallet;
use actix::prelude::*;
use actix_web::middleware::identity::{CookieIdentityPolicy, IdentityService};
use actix_web::middleware::session::{CookieSessionBackend, SessionStorage};
use actix_web::{http::Method, middleware, App};

pub struct AppState {
    pub db: Addr<DbExecutor>,
    pub wallet: Wallet,
    pub fsm: Addr<Fsm>,
}

pub fn create_app(
    db: Addr<DbExecutor>,
    wallet: Wallet,
    fsm: Addr<Fsm>,
    cookie_secret: &[u8],
) -> App<AppState> {
    App::with_state(AppState { db, wallet, fsm })
        .middleware(middleware::Logger::new("\"%r\" %s %b %Dms"))
        .middleware(IdentityService::new(
            CookieIdentityPolicy::new(cookie_secret)
                .name("auth-example")
                .secure(false),
        ))
        .middleware(SessionStorage::new(
            CookieSessionBackend::private(cookie_secret).secure(false),
        ))
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
        .resource(
            "/merchants/{merchant_id}/orders/{order_id}/{grin_path:.*}",
            |r| {
                r.method(Method::POST).with(pay_order);
            },
        )
        .resource("/tx", |r| r.method(Method::GET).with(get_tx))
        .resource("/login", |r| {
            r.method(Method::POST).with(login);
            r.method(Method::GET).with(login_form);
        })
        .resource("/logout", |r| r.method(Method::GET).with(logout))
        .resource("/", |r| {
            r.middleware(SiteAuthMiddleware);
            r.method(Method::GET).f(index);
        })
        .resource("/set_2fa", |r| {
            r.method(Method::GET).with(get_totp);
            r.method(Method::POST).with(post_totp);
        })
        .resource("/2fa", |r| {
            r.method(Method::GET).with(form_2fa);
            r.method(Method::POST).with(post_2fa);
        })
}
