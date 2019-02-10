use crate::schema::merchants;
use chrono::NaiveDateTime;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Serialize, Deserialize, Queryable, Insertable)]
#[table_name = "merchants"]
pub struct Merchant {
    pub id: Uuid,
    pub email: String,
    pub password: String,
    pub wallet_url: Option<String>,
    pub balance: i64,
    pub created_at: NaiveDateTime, // only NaiveDateTime works here due to diesel limitations
}

impl Merchant {
    // this is just a helper function to remove password from user just before we return the value out later
    pub fn remove_pwd(mut self) -> Self {
        self.password = "".to_string();
        self
    }
}
