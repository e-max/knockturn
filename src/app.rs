use crate::db::DbExecutor;
use crate::fsm::Fsm;
use crate::handlers::*;
use crate::wallet::Wallet;
use actix::prelude::*;
use actix_web::middleware::identity::{CookieIdentityPolicy, IdentityService};
use actix_web::middleware::session::{CookieSessionBackend, SessionStorage};
use actix_web::{http::Method, middleware, App};
use diesel::pg::PgConnection;
use diesel::r2d2::{ConnectionManager, Pool};
use sentry_actix::SentryMiddleware;

pub struct AppState {
    pub db: Addr<DbExecutor>,
    pub wallet: Wallet,
    pub pool: Pool<ConnectionManager<PgConnection>>,
    pub fsm: Addr<Fsm>,
}

pub fn create_app(
    db: Addr<DbExecutor>,
    wallet: Wallet,
    fsm: Addr<Fsm>,
    pool: Pool<ConnectionManager<PgConnection>>,
    cookie_secret: &[u8],
) -> App<AppState> {
    let state = AppState {
        db,
        wallet,
        fsm,
        pool,
    };
    App::with_state(state)
        .middleware(SentryMiddleware::new())
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
        .resource("/merchants/{merchant_id}/payments", |r| {
            r.method(Method::POST).with(payment::create_payment)
        })
        .resource("/merchants/{merchant_id}/payments/{transaction_id}", |r| {
            r.method(Method::GET).with(payment::get_payment);
            r.method(Method::POST).with(payment::make_payment);
        })
        .resource(
            "/merchants/{merchant_id}/payments/{transaction_id}/status",
            |r| {
                r.method(Method::GET).with(payment::get_payment_status);
            },
        )
        .resource(
            "/merchants/{merchant_id}/payments/{transaction_id}/{grin_path:.*}",
            |r| {
                r.method(Method::POST).with(payment::make_payment);
            },
        )
        .resource("/login", |r| {
            r.method(Method::POST).with(webui::login);
            r.method(Method::GET).with(webui::login_form);
        })
        .resource("/logout", |r| r.method(Method::POST).with(webui::logout))
        .resource("/", |r| {
            r.method(Method::GET).with(webui::index);
        })
        .resource("/set_2fa", |r| {
            r.method(Method::GET).with(mfa::get_totp);
            r.method(Method::POST).with(mfa::post_totp);
        })
        .resource("/2fa", |r| {
            r.method(Method::GET).with(mfa::form_2fa);
            r.method(Method::POST).with(mfa::post_2fa);
        })
        .resource("/withdraw", |r| {
            r.method(Method::GET).with(payout::withdraw);
            r.method(Method::POST).with(payout::create_payout);
        })
        .resource("/withdraw/confirm", |r| {
            r.method(Method::POST).with(payout::withdraw_confirmation);
        })
        .resource("/payouts/{id}", |r| {
            r.method(Method::GET).with(payout::get_payout);
            r.method(Method::POST).with(payout::accept_slate)
        })
        .resource("/payouts/{id}/knockturn-payout.grinslate", |r| {
            r.method(Method::GET).with(payout::generate_slate);
        })
        .resource("/transactions", |r| {
            r.method(Method::GET).with(webui::get_transactions)
        })
}
