use crate::errors::Error;
use actix_web::dev;
use actix_web::{FromRequest, HttpRequest};
use core::future::Future;
use diesel::query_dsl::methods::{LimitDsl, OffsetDsl};
use futures::future::FutureExt;
use serde::Deserialize;
use serde_urlencoded;
use std::borrow::Cow;
use std::cmp;
use std::fmt;
use std::pin::Pin;
use url::Url;

#[derive(Debug, Clone)]
pub struct Paginate {
    pub page: i64,
    pub per_page: i64,
    url: Url,
    max_pages: Option<i64>,
}

impl<'a> Paginate {
    pub fn for_total(&'a self, total_items: i64) -> Pages<'a> {
        let mut total_pages = total_items / self.per_page;
        if total_items % self.per_page != 0 {
            total_pages += 1;
        }

        let mut start = 1;
        let mut end = total_pages;

        if let Some(max) = self.max_pages {
            start = cmp::max(start, self.page - max / 2);
            end = start + max - 1;
            if end > total_pages {
                start = cmp::max(1, start - (end - total_pages));
                end = total_pages;
            }
        }

        Pages {
            page: self.page,
            start: start,
            end: end,
            total: total_pages,
            url: &self.url,
        }
    }
}

pub struct PaginateConfig {
    per_page: i64,
    max_pages: Option<i64>,
}

impl Default for PaginateConfig {
    fn default() -> Self {
        PaginateConfig {
            per_page: 10,
            max_pages: Some(10),
        }
    }
}

#[derive(Deserialize, Debug)]
struct PageInfo {
    page: Option<i64>,
    per_page: Option<i64>,
}

impl FromRequest for Paginate {
    type Config = PaginateConfig;
    type Future = Pin<Box<dyn Future<Output = Result<Self, Self::Error>>>>;
    type Error = Error;

    #[inline]
    fn from_request(req: &HttpRequest, _: &mut dev::Payload) -> Self::Future {
        let req = req.clone();
        async {
            let tmp;
            let cfg = if let Some(cfg) = req.app_data::<PaginateConfig>() {
                cfg
            } else {
                tmp = PaginateConfig::default();
                &tmp
            };
            let conn = req.connection_info();
            let url = Url::parse(&format!(
                "{}://{}{}?{}",
                conn.scheme(),
                conn.host(),
                req.path(),
                req.query_string(),
            ))
            .map_err(|e| Error::General(s!(e)))?;

            let page_info = serde_urlencoded::from_str::<PageInfo>(req.query_string())
                .map_err(|e| Error::General(s!(e)))?;

            Ok(Paginate {
                page: page_info.page.unwrap_or(1),
                per_page: page_info.per_page.unwrap_or(cfg.per_page),
                url: url,
                max_pages: cfg.max_pages,
            })
        }
        .boxed_local()
    }
}

pub struct PageIter<'a> {
    current: i64,
    pages: Cow<'a, Pages<'a>>,
}

#[derive(Debug, Clone)]
pub struct Pages<'a> {
    pub page: i64,
    pub start: i64,
    pub end: i64,
    pub total: i64,
    url: &'a Url,
}

impl<'a> Pages<'a> {
    pub fn url_for_page(&self, page: i64) -> String {
        let mut url = self.url.clone();

        url.query_pairs_mut()
            .clear()
            .extend_pairs(self.url.query_pairs().filter(|(k, _)| k != "page"));

        url.query_pairs_mut().append_pair("page", &page.to_string());

        url.as_str().to_owned()
    }

    pub fn get(&self, page: i64) -> Page {
        if page == self.page {
            Page {
                url: "".to_owned(),
                num: self.page.to_string(),
                is_current: true,
            }
        } else {
            Page {
                url: self.url_for_page(page),
                num: page.to_string(),
                is_current: false,
            }
        }
    }
}

impl<'a> IntoIterator for Pages<'a> {
    type Item = Page;
    type IntoIter = PageIter<'a>;
    fn into_iter(self) -> Self::IntoIter {
        PageIter {
            current: self.start - 1,
            pages: Cow::Owned(self),
        }
    }
}

impl<'a> IntoIterator for &'a Pages<'a> {
    type Item = Page;
    type IntoIter = PageIter<'a>;
    fn into_iter(self) -> Self::IntoIter {
        PageIter {
            current: self.start - 1,
            pages: Cow::Borrowed(&self),
        }
    }
}

pub struct Page {
    pub url: String,
    pub num: String,
    pub is_current: bool,
}
impl fmt::Display for Page {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        if self.is_current {
            write!(f, "{}", self.num)
        } else {
            write!(f, "<a href =\"{}\">{}</a>", self.url, self.num)
        }
    }
}

impl<'a> Iterator for PageIter<'a> {
    type Item = Page;
    fn next(&mut self) -> Option<Self::Item> {
        if self.current >= self.pages.end {
            return None;
        }
        self.current += 1;
        Some(self.pages.get(self.current))
    }
}

pub trait Paginator: Sized
where
    Self: OffsetDsl,
    Self::Output: LimitDsl,
{
    fn for_page(self, p: &Paginate) -> <Self::Output as LimitDsl>::Output {
        self.offset((p.page - 1) * p.per_page).limit(p.per_page)
    }
}

impl<T> Paginator for T
where
    T: OffsetDsl,
    T::Output: LimitDsl,
{
}

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
