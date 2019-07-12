use actix::prelude::*;
use actix_identity::{CookieIdentityPolicy, IdentityService};
use actix_session::CookieSession;
use actix_web::{middleware, App, HttpServer};
use diesel::pg::PgConnection;
use diesel::r2d2::{ConnectionManager, Pool};
use dotenv::dotenv;
use env_logger;
use knockturn::app::{routing, AppCfg, AppState};
use knockturn::db::DbExecutor;
use knockturn::fsm::Fsm;
use knockturn::fsm_payout::FsmPayout;
use knockturn::handlers::*;
use knockturn::node::Node;
use knockturn::wallet::Wallet;
use knockturn::{cron, cron_payout};
use log::info;
use rustls::internal::pemfile::{certs, pkcs8_private_keys};
use rustls::{NoClientAuth, ServerConfig};
use sentry;
use sentry_actix::SentryMiddleware;
use std::env;
use std::fs::File;
use std::io::BufReader;

#[macro_use]
extern crate diesel_migrations;

embed_migrations!();

fn main() {
    dotenv().ok();

    env_logger::init();

    let cookie_secret = env::var("COOKIE_SECRET").expect("COOKIE_SECRET must be set");
    let database_url = env::var("DATABASE_URL").expect("DATABASE_URL must be set");
    let host = env::var("HOST").unwrap_or("0.0.0.0:3000".to_owned());
    let _ = env::var("DOMAIN").expect("DOMAIN must be set");

    let wallet_url = env::var("WALLET_URL").expect("WALLET_URL must be set");
    let wallet_user = env::var("WALLET_USER").expect("WALLET_USER must be set");
    let wallet_pass = env::var("WALLET_PASS").expect("WALLET_PASS must be set");

    let node_url = env::var("NODE_URL").expect("NODE_URL must be set");
    let node_user = env::var("NODE_USER").expect("NODE_USER must be set");
    let node_pass = env::var("NODE_PASS").expect("NODE_PASS must be set");
    let sentry_url = env::var("SENTRY_URL").unwrap_or("".to_owned());

    if sentry_url != "" {
        let _ = sentry::init("https://3a46c4de68e54de9ab7e86e7547a4073@sentry.io/1464519");
        env::set_var("RUST_BACKTRACE", "1");
        sentry::integrations::panic::register_panic_handler();
    }

    let cfg = AppCfg {
        node_url,
        node_user,
        node_pass,
        wallet_url,
        wallet_user,
        wallet_pass,
        database_url,
    };

    info!("Starting");
    let sys = System::new("knockturn-server");

    let manager = ConnectionManager::<PgConnection>::new(cfg.database_url.as_str());
    let pool = r2d2::Pool::builder()
        .build(manager)
        .expect("Failed to create pool.");

    let conn: &PgConnection = &pool.get().unwrap();
    embedded_migrations::run_with_output(conn, &mut std::io::stdout());

    let db: Addr<DbExecutor> = SyncArbiter::start(10, {
        let pool = pool.clone();
        move || DbExecutor(pool.clone())
    });
    let wallet = Wallet::new(&cfg.wallet_url, &cfg.wallet_user, &cfg.wallet_pass);
    let node = Node::new(&cfg.node_url, &cfg.node_user, &cfg.node_pass);
    let fsm: Addr<Fsm> = Fsm {
        db: db.clone(),
        wallet: wallet.clone(),
        pool: pool.clone(),
    }
    .start();

    let fsm_payout: Addr<FsmPayout> = FsmPayout {
        db: db.clone(),
        wallet: wallet.clone(),
        pool: pool.clone(),
    }
    .start();

    cron::Cron::new(db.clone(), fsm.clone(), node, pool.clone()).start();
    cron_payout::CronPayout::new(db.clone(), fsm_payout.clone(), pool.clone()).start();

    let mut srv = HttpServer::new({
        let pool = pool.clone();
        move || {
            let mut app = App::new()
                .data(AppState {
                    db: db.clone(),
                    wallet: wallet.clone(),
                    pool: pool.clone(),
                    fsm: fsm.clone(),
                    fsm_payout: fsm_payout.clone(),
                })
                .configure(routing)
                .wrap(middleware::Logger::new("\"%r\" %s %b %Dms"))
                .wrap(IdentityService::new(
                    CookieIdentityPolicy::new(cookie_secret.as_bytes())
                        .name("auth-example")
                        .secure(false),
                ))
                .wrap(CookieSession::private(cookie_secret.as_bytes()).secure(false));

            /*
             * doesn't work yet with actix 1.0
             * https://github.com/getsentry/sentry-rust/issues/143
             *
            if sentry_url != "" {
                app = app.wrap(SentryMiddleware::new());
            }
            */
            app
        }
    });

    if let Ok(folder) = env::var("TLS_FOLDER") {
        // load ssl keys
        let mut config = ServerConfig::new(NoClientAuth::new());
        let cert_file =
            &mut BufReader::new(File::open(format!("{}/fullchain.pem", folder)).unwrap());
        let key_file = &mut BufReader::new(File::open(format!("{}/privkey.pem", folder)).unwrap());
        let cert_chain = certs(cert_file).unwrap();
        let mut keys = pkcs8_private_keys(key_file).unwrap();
        config.set_single_cert(cert_chain, keys.remove(0)).unwrap();
        srv.bind_rustls(&host, config)
            .expect(&format!("Can not TLS  bind to '{}'", &host))
            .start();
    } else {
        srv.bind(&host)
            .expect(&format!("Can not bind to '{}'", &host))
            .start();
    }

    sys.run().unwrap();
}
