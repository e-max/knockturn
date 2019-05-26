use diesel::dsl::Limit;
use diesel::query_dsl::methods::{LimitDsl, OffsetDsl};
use diesel::query_dsl::QueryDsl;
use serde::Deserialize;

#[derive(Deserialize)]
pub struct Paginate {
    #[serde(default)]
    page: i64,
    #[serde(default)]
    per_page: i64,
}

impl Default for Paginate {
    fn default() -> Self {
        Paginate {
            page: 1,
            per_page: 10,
        }
    }
}

impl Paginate {
    pub fn set<T>(&self, query: T) -> Limit<T::Output>
    where
        T: OffsetDsl,
        T::Output: LimitDsl,
        <<T as OffsetDsl>::Output as LimitDsl>::Output: QueryDsl,
    {
        let page = if self.page == 0 { 1 } else { self.page };
        let per_page = if self.per_page == 0 {
            10
        } else {
            self.per_page
        };

        LimitDsl::limit(OffsetDsl::offset(query, (page - 1) * per_page), per_page)
    }
}
