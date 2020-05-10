use crate::app::AppState;
use crate::db::{Confirm2FA, GetMerchant};
use crate::errors::*;
use crate::extractor::Session;
use crate::handlers::TemplateIntoResponse;
use crate::models::Merchant;
use crate::totp::Totp;
use actix_identity::Identity;
use actix_session::Session as ActixSession;
use actix_web::http::Method;
use actix_web::web::{Data, Form};
use actix_web::{HttpRequest, HttpResponse};
use askama::Template;
use data_encoding::BASE64;
use serde::Deserialize;

#[derive(Template)]
#[template(path = "totp.html")]
struct TotpTemplate<'a> {
    msg: &'a str,
    token: &'a str,
    image: &'a str,
}

#[derive(Debug, Deserialize)]
pub struct TotpRequest {
    pub code: String,
}

#[derive(Template)]
#[template(path = "2fa.html")]
struct TwoFATemplate;

pub async fn form_2fa(_: HttpRequest) -> Result<HttpResponse, Error> {
    TwoFATemplate {}.into_response()
}

pub async fn get_totp(merchant: Session<Merchant>) -> Result<HttpResponse, Error> {
    let merchant = merchant.into_inner();
    let token = merchant
        .token_2fa
        .ok_or(Error::General(s!("No 2fa token")))?;
    let totp = Totp::new(merchant.id.clone(), token.clone());

    let html = TotpTemplate {
        msg: "",
        token: &token,
        image: &BASE64.encode(&totp.get_png()?),
    }
    .render()
    .map_err(|e| Error::from(e))?;
    Ok(HttpResponse::Ok().content_type("text/html").body(html))
}

pub async fn post_totp(
    merchant: Session<Merchant>,
    req: HttpRequest,
    totp_form: Form<TotpRequest>,
    data: Data<AppState>,
) -> Result<HttpResponse, Error> {
    let merchant = merchant.into_inner();
    let mut msg = String::new();

    let token = match merchant.token_2fa {
        Some(t) => t,
        None => return Err(Error::General(s!("No 2fa token"))),
    };
    let totp = Totp::new(merchant.id.clone(), token.clone());

    if req.method() == Method::POST {
        match totp.check(&totp_form.code) {
            Ok(true) => {
                let resp = HttpResponse::Found().header("location", "/").finish();
                data.db
                    .send(Confirm2FA {
                        merchant_id: merchant.id,
                    })
                    .await??;
                return Ok(resp);
            }
            _ => msg.push_str("Incorrect code, please try one more time"),
        }
    }

    let image = match totp.get_png() {
        Err(_) => return Err(Error::General(s!("can't generate an image"))),
        Ok(v) => v,
    };

    let html = match (TotpTemplate {
        msg: &msg,
        token: &token,
        image: &BASE64.encode(&image),
    }
    .render())
    {
        Err(e) => return Err(Error::from(e)),
        Ok(v) => v,
    };
    Ok(HttpResponse::Ok().content_type("text/html").body(html))
}

pub async fn post_2fa(
    session: ActixSession,
    totp_form: Form<TotpRequest>,
    data: Data<AppState>,
    identity: Identity,
) -> Result<HttpResponse, Error> {
    let merchant_id = match session.get::<String>("merchant") {
        Ok(Some(v)) => v,
        _ => {
            return Ok(HttpResponse::Found().header("location", "/login").finish());
        }
    };
    let merchant = data
        .db
        .send(GetMerchant {
            id: merchant_id.clone(),
        })
        .await??;

    let token = merchant
        .token_2fa
        .ok_or(Error::General(s!("No 2fa token")))?;
    let totp = Totp::new(merchant.id.clone(), token.clone());

    if totp.check(&totp_form.code)? {
        identity.remember(merchant.id);
        Ok(HttpResponse::Found().header("location", "/").finish())
    } else {
        Ok(HttpResponse::Found().header("location", "/2fa").finish())
    }
}
