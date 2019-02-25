use actix_web::client::ClientRequestBuilder;
use actix_web::http::header;
use base64::encode;

pub trait PlainHttpAuth {
    fn auth(&mut self, username: &str, password: &str) -> &mut Self;
}

impl PlainHttpAuth for ClientRequestBuilder {
    fn auth(&mut self, username: &str, password: &str) -> &mut Self {
        let auth = format!("{}:{}", username, password);
        let auth_header = format!("Basic {}", encode(&auth));
        self.header(header::AUTHORIZATION, auth_header)
    }
}
