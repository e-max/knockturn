use diesel::query_dsl::methods::{LimitDsl, OffsetDsl};
use serde::Deserialize;
use url::Url;
use std::rc::Rc;
use actix_web::{Error, HttpRequest, FromRequest, HttpResponse, ResponseError};
use actix_web::dev::QueryConfig;
use serde_urlencoded;
use failure::Fail;
use actix_web::http::StatusCode;
use std::fmt;
use std::str::FromStr;
use failure::ResultExt;


#[derive(Fail, Debug)]
pub struct PaginateError;

impl fmt::Display for PaginateError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        fmt::Display::fmt("paginate error", f)
    }
}

    
impl ResponseError for PaginateError {
    fn error_response(&self) -> HttpResponse {
        HttpResponse::new(StatusCode::BAD_REQUEST)
    }
}





#[derive(Debug, Clone)]
pub struct Paginate {
    pub page: i64,
    pub per_page: i64,
    url: Url,
}


impl Paginate {
    pub fn for_total(&self, total: i64) ->  Pages {
        Pages{page: self.page, total: total, url: self.url.clone()}
    }
}

pub struct PaginateConfig {
    per_page: i64,
}

impl Default for PaginateConfig {
    fn default() -> Self {
        PaginateConfig {
            per_page: 10,
        }
    }
}

#[derive(Deserialize, Debug)]
pub struct PageInfo {
    page: Option<i64>,
    per_page: Option<i64>,
}


impl<S> FromRequest<S> for Paginate
{
    type Config = PaginateConfig;
    type Result = Result<Self, Error>;

    #[inline]
    fn from_request(req: &HttpRequest<S>, cfg: &Self::Config) -> Self::Result {
        let conn = req.connection_info();
        let url = Url::parse(&format!(
            "{}://{}{}",
            conn.scheme(),
            conn.host(),
            req.path(),
        ))?;
        let q = req.query();
        for (k, v) in q.iter() {
            println!("\x1B[31;1m k=v\x1B[0m = {:?}={:?}", k,v);
        }


        let page_info = serde_urlencoded::from_str::<PageInfo>(req.query_string())?;
        
        Ok(Paginate{
            page :page_info.page.unwrap_or(1), 
            per_page: page_info.per_page.unwrap_or(cfg.per_page), 
            url: url
        })
    }
}


pub struct PageIter {
    page: i64,
    total: i64,
    current: i64,
    url: Url,
}

pub struct Pages {
    page: i64,
    total: i64,
    url: Url,
}

impl IntoIterator for Pages {
    type Item=String;
    type IntoIter = PageIter;
    fn into_iter(self) -> Self::IntoIter {
        PageIter {
            page: self.page,
            total: self.total,
            current: 0,
            url: self.url.clone(),
        }
    }
}

impl<'a> IntoIterator for &'a Pages {
    type Item=String;
    type IntoIter = PageIter;
    fn into_iter(self) -> Self::IntoIter {
        PageIter {
            page: self.page,
            total: self.total,
            current: 0,
            url: self.url.clone(),
        }
    }
}

impl Iterator for PageIter {
    type Item=String;
    fn next(&mut self) ->  Option<String> {
        if self.current >=self.total {
            return None
        }
        self.current+=1;
        Some(format!("current={}", self.current))
    }
}


pub trait Paginator : Sized
where
    Self: OffsetDsl,
    Self::Output: LimitDsl
{
    fn for_page(self, p: &Paginate) -> <Self::Output as LimitDsl>::Output {
        self.offset(p.page - 1).limit(p.per_page)
    }
}

impl<T> Paginator for T
where
    T: OffsetDsl,
    T::Output: LimitDsl {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn url_parser_test() {
        dbg!(Url::parse("data:text/plain,Hello?World#"));
        dbg!(Url::parse("data:text/plain,Hello"));
        dbg!(Url::parse("data:text/plain"));
        dbg!(Url::parse("data:/plain"));
        dbg!(Url::parse("localhost:/plain"));
        
    }
}
