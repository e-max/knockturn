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

pub trait BearerTokenAuth {
    fn bearer_token(&mut self, token: &str) -> &mut Self;
}

impl BearerTokenAuth for ClientRequestBuilder {
    fn bearer_token(&mut self, token: &str) -> &mut Self {
        let auth_header = format!("Bearer {}", token);
        self.header(header::AUTHORIZATION, auth_header)
    }
}

//pub trait BasicAuthUser {
//    fn auth_user(&mut self) -> Box<Future<Item = Merchant, Error = Error>>;
//}
//
//impl BasicAuthUser for HttpRequest<AppState> {
//    fn auth_user(&mut self) -> Box<Future<Item = Merchant, Error = Error>> {
//        let base = match self.headers().get("Authentication") {
//            None => return Box::new(err(Error::AuthRequired)),
//            Some(val) => decode(val).map_err(|| Box::new(err(Error::AuthRequired)))?,
//        };
//        let (user, password, _) = String::from_utf8_lossy(&base).split(':');
//    }
//}
