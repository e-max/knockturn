use actix_web::{error::ResponseError, HttpResponse};

#[derive(Fail, Debug)]
pub enum Error {
    #[fail(display = "DB Error: {}", _0)]
    Db(String),

    #[fail(display = "Entity not found: {}", _0)]
    EntityNotFound(String),

    #[fail(display = "Invalid entity {}", _0)]
    InvalidEntity(String),

    #[fail(display = "Entity already exists {}", _0)]
    AlreadyExists(String),

    #[fail(display = "Template erorr")]
    Template(String),

    #[fail(display = "Unsupported currency: {}", _0)]
    UnsupportedCurrency(String),

    #[fail(display = "General error: {}", _0)]
    General(String),

    #[fail(display = "Got error when call wallet API {}", _0)]
    WalletAPIError(String),
}

impl From<diesel::result::Error> for Error {
    fn from(error: diesel::result::Error) -> Self {
        match error {
            diesel::result::Error::NotFound => Error::EntityNotFound(format!("Not found")),
            diesel::result::Error::DatabaseError(kind, _) => match kind {
                diesel::result::DatabaseErrorKind::UniqueViolation
                | diesel::result::DatabaseErrorKind::ForeignKeyViolation => {
                    Error::AlreadyExists("Already exists".to_owned())
                }
                _ => Error::Db(format!("{:?}", error)),
            },
            _ => Error::Db(format!("{:?}", error)),
        }
    }
}

impl From<askama::Error> for Error {
    fn from(error: askama::Error) -> Self {
        Error::Template(format!("{:?}", error))
    }
}

impl From<serde_json::error::Error> for Error {
    fn from(error: serde_json::error::Error) -> Self {
        Error::General(format!("{:?}", error))
    }
}

impl From<std::str::Utf8Error> for Error {
    fn from(error: std::str::Utf8Error) -> Self {
        Error::General(format!("{:?}", error))
    }
}

// impl ResponseError trait allows to convert our errors into http responses with appropriate data
impl ResponseError for Error {
    fn error_response(&self) -> HttpResponse {
        match *self {
            Error::Db(ref message) | Error::Template(ref message) => {
                HttpResponse::InternalServerError().json(message)
            }
            Error::EntityNotFound(ref message) => HttpResponse::NotFound().json(message),
            Error::InvalidEntity(ref message)
            | Error::AlreadyExists(ref message)
            | Error::UnsupportedCurrency(ref message) => HttpResponse::BadRequest().json(message),
            _ => HttpResponse::InternalServerError().json("general error".to_owned()),
        }
    }
}
