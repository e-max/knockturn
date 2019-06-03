use actix_web::{Error, FromRequest, HttpRequest};
use diesel::query_dsl::methods::{LimitDsl, OffsetDsl};
use serde::Deserialize;
use serde_urlencoded;
use std::fmt;
use url::Url;

#[derive(Debug, Clone)]
pub struct Paginate {
    pub page: i64,
    pub per_page: i64,
    url: Url,
}

impl<'a> Paginate {
    pub fn for_total(&'a self, total_items: i64) -> Pages<'a> {
        let mut total_pages = total_items / self.per_page;
        if total_items % self.per_page != 0 {
            total_pages += 1;
        }
        Pages {
            page: self.page,
            total: total_pages,
            url: &self.url,
        }
    }
}

pub struct PaginateConfig {
    per_page: i64,
}

impl Default for PaginateConfig {
    fn default() -> Self {
        PaginateConfig { per_page: 10 }
    }
}

#[derive(Deserialize, Debug)]
struct PageInfo {
    page: Option<i64>,
    per_page: Option<i64>,
}

impl<S> FromRequest<S> for Paginate {
    type Config = PaginateConfig;
    type Result = Result<Self, Error>;

    #[inline]
    fn from_request(req: &HttpRequest<S>, cfg: &Self::Config) -> Self::Result {
        let conn = req.connection_info();
        let url = Url::parse(&format!(
            "{}://{}{}?{}",
            conn.scheme(),
            conn.host(),
            req.path(),
            req.query_string(),
        ))?;

        let page_info = serde_urlencoded::from_str::<PageInfo>(req.query_string())?;

        Ok(Paginate {
            page: page_info.page.unwrap_or(1),
            per_page: page_info.per_page.unwrap_or(cfg.per_page),
            url: url,
        })
    }
}

pub struct PageIter<'a> {
    page: i64,
    total: i64,
    current: i64,
    url: &'a Url,
}

pub struct Pages<'a> {
    page: i64,
    total: i64,
    url: &'a Url,
}

impl<'a> IntoIterator for Pages<'a> {
    type Item = Page;
    type IntoIter = PageIter<'a>;
    fn into_iter(self) -> Self::IntoIter {
        PageIter {
            page: self.page,
            total: self.total,
            current: 0,
            url: self.url,
        }
    }
}

impl<'a> IntoIterator for &'a Pages<'a> {
    type Item = Page;
    type IntoIter = PageIter<'a>;
    fn into_iter(self) -> Self::IntoIter {
        PageIter {
            page: self.page,
            total: self.total,
            current: 0,
            url: self.url,
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
        if self.current >= self.total {
            return None;
        }
        self.current += 1;
        let mut url = self.url.clone();

        url.query_pairs_mut()
            .clear()
            .extend_pairs(self.url.query_pairs().filter(|(k, _)| k != "page"));

        url.query_pairs_mut()
            .append_pair("page", &self.current.to_string());

        if self.current == self.page {
            Some(Page {
                url: "".to_owned(),
                num: self.page.to_string(),
                is_current: true,
            })
        } else {
            Some(Page {
                url: url.as_str().to_owned(),
                num: self.current.to_string(),
                is_current: false,
            })
        }
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
