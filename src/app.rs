use crate::db::{get_current_height, DbExecutor};
use crate::errors::Error;
use crate::fsm::Fsm;
use crate::fsm_payout::FsmPayout;
use crate::handlers::*;
use crate::node::Node;
use crate::wallet::Wallet;
use crate::Pool;
use actix::prelude::*;
use actix_web::web;
use diesel::pg::PgConnection;
use diesel::{self, prelude::*};
use futures::future::Future;
use log::*;

const HORIZON_HEIGHT: i64 = 60 * 24 * 5; // approximate number of blocks generated in 5 days

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
    pub pool: Pool,
    pub fsm: Addr<Fsm>,
    pub fsm_payout: Addr<FsmPayout>,
}

pub async fn check_node_horizon(node: &Node, pool: &Pool) -> Result<(), Error> {
    info!("Try to check how differ height on node and in DB");
    let pool = pool.clone();

    let node_height = node.current_height().await?;

    let _ = web::block::<_, _, Error>({
        let pool = pool.clone();
        move || {
            let conn: &PgConnection = &pool.get().unwrap();
            let last_height = get_current_height(conn).or_else(|e| match e {
                Error::EntityNotFound(_) => Ok(0),
                _ => Err(e),
            })?;
            info!(
                "Node height: {}, local height: {}",
                node_height, last_height
            );
            if node_height - last_height > HORIZON_HEIGHT {
                warn!(
                    "Current height {} is outdated! Reset to current node height {}",
                    last_height, node_height
                );
                use crate::schema::current_height::dsl::*;
                diesel::update(current_height)
                    .set(height.eq(node_height as i64))
                    .execute(conn)
                    .map(|_| ())
                    .map_err::<Error, _>(|e| e.into())?;
            }
            Ok(())
        }
    })
    .await?;

    Ok(())
}

pub fn routing(cfg: &mut web::ServiceConfig) {
    cfg.service(web::resource("/merchants").route(web::post().to(create_merchant)))
        .service(web::resource("/merchants/{merchant_id}").route(web::get().to(get_merchant)))
        .service(
            web::resource("/merchants/{merchant_id}/payments")
                .route(web::post().to(payment::create_payment)),
        )
        .service(
            web::resource("/merchants/{merchant_id}/payments/{transaction_id}")
                .route(web::get().to(payment::get_payment))
                .route(web::post().to(payment::make_payment)),
        )
        .service(
            web::resource("/merchants/{merchant_id}/payments/{transaction_id}/status")
                .route(web::get().to(payment::get_payment_status)),
        )
        .service(
            web::resource("/merchants/{merchant_id}/payments/{transaction_id}/{grin_path:.*}")
                .route(web::post().to(payment::wallet_jsonrpc)),
        )
        .service(
            web::resource("/login")
                .route(web::post().to(webui::login))
                .route(web::get().to(webui::login_form)),
        )
        .service(web::resource("/logout").route(web::post().to(webui::logout)))
        .service(web::resource("/").route(web::get().to(webui::index)))
        .service(
            web::resource("/set_2fa")
                .route(web::get().to(mfa::get_totp))
                .route(web::post().to(mfa::post_totp)),
        )
        .service(
            web::resource("/2fa")
                .route(web::get().to(mfa::form_2fa))
                .route(web::post().to(mfa::post_2fa)),
        )
        .service(
            web::resource("/withdraw")
                .route(web::get().to(payout::withdraw))
                .route(web::post().to(payout::create_payout)),
        )
        .service(
            web::resource("/withdraw/confirm").route(web::post().to(payout::withdraw_confirmation)),
        )
        .service(
            web::resource("/payouts/{id}")
                .route(web::get().to(payout::get_payout))
                .route(web::post().to(payout::accept_slate)),
        )
        .service(
            web::resource("/payouts/{id}/knockturn-payout.grinslate")
                .route(web::get().to(payout::generate_slate)),
        )
        .service(
            web::resource("/transactions/{id}").route(web::get().to(transaction::get_transaction)),
        )
        .service(
            web::resource("/transactions/{id}/status_history")
                .route(web::get().to(transaction::get_transaction_status_changes)),
        )
        .service(
            web::resource("/transactions/{id}/manually_refunded")
                .route(web::post().to(transaction::manually_refunded)),
        )
        .service(
            web::resource("/transactions").route(web::get().to(transaction::get_transactions)),
        );
}
