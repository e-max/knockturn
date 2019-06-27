use actix_identity::{CookieIdentityPolicy, IdentityService};
use actix_session::CookieSession;
use actix_web::{middleware, App, HttpServer};
use dotenv::dotenv;
use env_logger;
use knockturn::app::{routing, AppCfg, AppState};
use log::info;
use rustls::internal::pemfile::{certs, pkcs8_private_keys};
use rustls::{NoClientAuth, ServerConfig};
use sentry;
use sentry_actix::SentryMiddleware;
use std::env;
use std::fs::File;
use std::io::BufReader;

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

    let mut srv = HttpServer::new(move || {
        let mut app = App::new()
            .data(AppState::new(cfg.clone()))
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
            .run();
    } else {
        srv.bind(&host)
            .expect(&format!("Can not bind to '{}'", &host))
            .run();
    }

    /*
     *
     * replace me with a proper TLS implementation
    srv = if let Ok(folder) = env::var("TLS_FOLDER") {
        let mut builder = SslAcceptor::mozilla_intermediate(SslMethod::tls()).unwrap();
        builder
            .set_private_key_file(format!("{}/privkey.pem", folder), SslFiletype::PEM)
            .unwrap();
        builder
            .set_certificate_chain_file(format!("{}/fullchain.pem", folder))
            .unwrap();
        srv.bind_ssl(&host, builder)
            .expect(&format!("Can not bind_ssl to '{}'", &host))
    } else {
        srv.bind(&host)
            .expect(&format!("Can not bind to '{}'", &host))
    };
    */
}
