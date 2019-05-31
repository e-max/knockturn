use diesel::query_dsl::methods::{LimitDsl, OffsetDsl};
use serde::Deserialize;

#[derive(Deserialize, Debug)]
#[serde(default)]
pub struct PageInfo {
    page: i64,
    per_page: i64,
}

impl Default for PageInfo {
    fn default() -> Self {
        PageInfo {
            page: 1,
            per_page: 10,
        }
    }
}

pub trait Paginator : Sized
where
    Self: OffsetDsl,
    Self::Output: LimitDsl
{
    fn for_page(self, p: &PageInfo) -> <Self::Output as LimitDsl>::Output {
        self.offset(p.page - 1).limit(p.per_page)
    }
}

impl<T> Paginator for T
where
    T: OffsetDsl,
    T::Output: LimitDsl {}

