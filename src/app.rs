use crate::db::DbExecutor;
use crate::fsm::Fsm;
use crate::fsm_payout::FsmPayout;
use crate::handlers::*;
use crate::node::Node;
use crate::wallet::Wallet;
use crate::{cron, cron_payout};
use actix::prelude::*;
use actix_web::web;
use diesel::pg::PgConnection;
use diesel::r2d2::{ConnectionManager, Pool};

#[derive(Debug, Clone)]
pub struct AppCfg {
    pub node_url: String,
    pub node_user: String,
    pub node_pass: String,
    pub wallet_url: String,
    pub wallet_user: String,
    pub wallet_pass: String,
    pub database_url: String,
}

pub struct AppState {
    pub db: Addr<DbExecutor>,
    pub wallet: Wallet,
    pub pool: Pool<ConnectionManager<PgConnection>>,
    pub fsm: Addr<Fsm>,
    pub fsm_payout: Addr<FsmPayout>,
}

impl AppState {
    pub fn new(cfg: AppCfg) -> Self {
        let manager = ConnectionManager::<PgConnection>::new(cfg.database_url.as_str());
        let pool = r2d2::Pool::builder()
            .build(manager)
            .expect("Failed to create pool.");
        let pool_clone = pool.clone();
        let db: Addr<DbExecutor> = SyncArbiter::start(10, move || DbExecutor(pool_clone.clone()));
        let wallet = Wallet::new(&cfg.wallet_url, &cfg.wallet_user, &cfg.wallet_pass);
        let node = Node::new(&cfg.node_url, &cfg.node_user, &cfg.node_pass);
        let cron_db = db.clone();
        let fsm: Addr<Fsm> = {
            let wallet = wallet.clone();
            let db = db.clone();
            let pool = pool.clone();
            Fsm { db, wallet, pool }.start()
        };

        let fsm_payout: Addr<FsmPayout> = {
            let wallet = wallet.clone();
            let db = db.clone();
            let pool = pool.clone();
            FsmPayout { db, wallet, pool }.start()
        };
        let _cron = {
            let fsm = fsm.clone();
            let pool = pool.clone();
            let cron_db = cron_db.clone();
            cron::Cron::new(cron_db, fsm, node, pool)
        }
        .start();
        let _cron_payout = {
            let fsm = fsm_payout.clone();
            cron_payout::CronPayout::new(cron_db.clone(), fsm, pool.clone())
        }
        .start();
        AppState {
            db,
            wallet,
            pool,
            fsm,
            fsm_payout,
        }
    }
}

pub fn routing(cfg: &mut web::ServiceConfig) {
    cfg.service(web::resource("/merchants").route(web::post().to_async(create_merchant)))
        .service(web::resource("/merchants/{merchant_id}").route(web::get().to_async(get_merchant)))
        .service(
            web::resource("/merchants/{merchant_id}/payments")
                .route(web::post().to_async(payment::create_payment)),
        )
        .service(
            web::resource("/merchants/{merchant_id}/payments/{transaction_id}")
                .route(web::get().to_async(payment::get_payment))
                .route(web::post().to_async(payment::make_payment)),
        )
        .service(
            web::resource("/merchants/{merchant_id}/payments/{transaction_id}/status")
                .route(web::get().to_async(payment::get_payment_status)),
        )
        .service(
            web::resource("/merchants/{merchant_id}/payments/{transaction_id}/{grin_path:.*}")
                .route(web::post().to_async(payment::make_payment)),
        )
        .service(
            web::resource("/login")
                .route(web::post().to_async(webui::login))
                .route(web::get().to(webui::login_form)),
        )
        .service(web::resource("/logout").route(web::post().to(webui::logout)))
        .service(web::resource("/").route(web::get().to_async(webui::index)))
        .service(
            web::resource("/set_2fa")
                .route(web::get().to(mfa::get_totp))
                .route(web::post().to(mfa::post_totp)),
        )
        .service(
            web::resource("/2fa")
                .route(web::get().to(mfa::form_2fa))
                .route(web::post().to_async(mfa::post_2fa)),
        )
        .service(
            web::resource("/withdraw")
                .route(web::get().to_async(payout::withdraw))
                .route(web::post().to_async(payout::create_payout)),
        )
        .service(
            web::resource("/withdraw/confirm").route(web::post().to(payout::withdraw_confirmation)),
        )
        .service(
            web::resource("/payouts/{id}")
                .route(web::get().to_async(payout::get_payout))
                .route(web::post().to_async(payout::accept_slate)),
        )
        .service(
            web::resource("/payouts/{id}/knockturn-payout.grinslate")
                .route(web::get().to_async(payout::generate_slate)),
        )
        .service(
            web::resource("/transactions/{id}")
                .route(web::get().to_async(transaction::get_transaction)),
        )
        .service(
            web::resource("/transactions/{id}/manually_refunded")
                .route(web::post().to_async(transaction::manually_refunded)),
        )
        .service(
            web::resource("/transactions")
                .route(web::get().to_async(transaction::get_transactions)),
        );
}
