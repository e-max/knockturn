use actix_web::{error::ResponseError, HttpResponse};

#[derive(Fail, Debug)]
pub enum Error {
    #[fail(display = "DB Error")]
    DB,

    #[fail(display = "Entity not found: {}", _0)]
    EntityNotFound(String),
}

// impl ResponseError trait allows to convert our errors into http responses with appropriate data
impl ResponseError for Error {
    fn error_response(&self) -> HttpResponse {
        match *self {
            Error::DB => HttpResponse::InternalServerError().json("Internal Server Error"),
            Error::EntityNotFound(ref message) => HttpResponse::NotFound().json(message),
        }
    }
}
