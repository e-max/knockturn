use crate::errors::*;
use crate::models::{Currency, Merchant, Money, Order, OrderStatus, Rate};
use actix::{Actor, SyncContext};
use actix::{Handler, Message};
use chrono::NaiveDateTime;
use chrono::{Duration, Local, Utc};
use data_encoding::BASE32;
use diesel::pg::PgConnection;
use diesel::r2d2::{ConnectionManager, Pool};
use diesel::{self, prelude::*};
use log::info;
use rand::seq::SliceRandom;
use rand::{thread_rng, Rng};
use serde::Deserialize;
use std::collections::HashMap;
use uuid::Uuid;

const MAX_REPORT_ATTEMPTS: i32 = 10; //Number or attemps we try to run merchant's callback

pub struct DbExecutor(pub Pool<ConnectionManager<PgConnection>>);

impl Actor for DbExecutor {
    type Context = SyncContext<Self>;
}

#[derive(Debug, Deserialize)]
pub struct CreateMerchant {
    pub id: String,
    pub email: String,
    pub password: String,
    pub wallet_url: Option<String>,
    pub callback_url: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct GetMerchant {
    pub id: String,
}

#[derive(Debug, Deserialize)]
pub struct GetOrder {
    pub order_id: Uuid,
}

#[derive(Debug, Deserialize)]
pub struct GetOrders {
    pub merchant_id: String,
    pub offset: i64,
    pub limit: i64,
}

#[derive(Debug, Deserialize)]
pub struct CreateOrder {
    pub merchant_id: String,
    pub external_id: String,
    pub amount: Money,
    pub confirmations: i32,
    pub email: Option<String>,
    pub message: String,
}

#[derive(Debug, Deserialize)]
pub struct UpdateOrderStatus {
    pub id: Uuid,
    pub status: OrderStatus,
}

#[derive(Debug, Deserialize)]
pub struct RegisterRate {
    pub rates: HashMap<String, f64>,
}

#[derive(Debug, Deserialize)]
pub struct ConvertCurrency {
    pub amount: Money,
    pub to: String,
}

#[derive(Debug, Deserialize)]
pub struct GetPendingOrders;

#[derive(Debug, Deserialize)]
pub struct GetConfirmedOrders;

pub struct ConfirmOrder {
    pub order: Order,
    pub confirmed_at: Option<NaiveDateTime>,
}

#[derive(Debug, Deserialize)]
pub struct ReportAttempt {
    pub order_id: Uuid,
    pub next_attempt: Option<NaiveDateTime>,
}

#[derive(Debug, Deserialize)]
pub struct MarkAsReported {
    pub order_id: Uuid,
}

#[derive(Debug, Deserialize)]
pub struct GetUnreportedOrders;

#[derive(Debug, Deserialize)]
pub struct Confirm2FA {
    pub merchant_id: String,
}

#[derive(Debug, Deserialize)]
pub struct Reset2FA {
    pub merchant_id: String,
}

#[derive(Debug, Deserialize)]
pub struct UpdateOrderWithTxLog {
    pub order_id: Uuid,
    pub wallet_tx_id: i64,
    pub wallet_tx_slate_id: String,
    pub messages: Option<Vec<String>>,
}

impl Message for CreateMerchant {
    type Result = Result<Merchant, Error>;
}

impl Message for GetMerchant {
    type Result = Result<Merchant, Error>;
}

impl Message for GetOrder {
    type Result = Result<Order, Error>;
}

impl Message for GetOrders {
    type Result = Result<Vec<Order>, Error>;
}

impl Message for CreateOrder {
    type Result = Result<Order, Error>;
}

impl Message for UpdateOrderStatus {
    type Result = Result<Order, Error>;
}

impl Message for RegisterRate {
    type Result = Result<(), Error>;
}

impl Message for ConvertCurrency {
    type Result = Result<Money, Error>;
}
impl Message for ConfirmOrder {
    type Result = Result<(), Error>;
}

impl Message for GetPendingOrders {
    type Result = Result<Vec<Order>, Error>;
}

impl Message for GetConfirmedOrders {
    type Result = Result<Vec<Order>, Error>;
}

impl Message for ReportAttempt {
    type Result = Result<(), Error>;
}

impl Message for MarkAsReported {
    type Result = Result<(), Error>;
}

impl Message for GetUnreportedOrders {
    type Result = Result<Vec<Order>, Error>;
}

impl Message for Confirm2FA {
    type Result = Result<(), Error>;
}

impl Message for Reset2FA {
    type Result = Result<(), Error>;
}

impl Message for UpdateOrderWithTxLog {
    type Result = Result<(), Error>;
}

impl Handler<CreateMerchant> for DbExecutor {
    type Result = Result<Merchant, Error>;

    fn handle(&mut self, msg: CreateMerchant, _: &mut Self::Context) -> Self::Result {
        use crate::schema::merchants::dsl::*;
        let conn: &PgConnection = &self.0.get().unwrap();
        const CHARSET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZ\
    abcdefghijklmnopqrstuvwxyz\
    0123456789-._~+/";

        let mut rng = thread_rng();
        let new_token: Option<String> = (0..64)
            .map(|_| Some(*CHARSET.choose(&mut rng)? as char))
            .collect();
        let new_token_2fa = BASE32.encode(&rng.gen::<[u8; 10]>());
        let new_merchant = Merchant {
            id: msg.id,
            email: msg.email,
            password: msg.password,
            wallet_url: msg.wallet_url,
            balance: 0,
            created_at: Local::now().naive_local() + Duration::hours(24),
            callback_url: msg.callback_url,
            token: new_token.ok_or(Error::General(s!("cannot generate rangom token")))?,
            token_2fa: Some(new_token_2fa),
            confirmed_2fa: false,
        };

        diesel::insert_into(merchants)
            .values(&new_merchant)
            .get_result(conn)
            .map_err(|e| e.into())
    }
}

impl Handler<GetMerchant> for DbExecutor {
    type Result = Result<Merchant, Error>;

    fn handle(&mut self, msg: GetMerchant, _: &mut Self::Context) -> Self::Result {
        use crate::schema::merchants::dsl::*;
        let conn: &PgConnection = &self.0.get().unwrap();
        merchants
            .find(msg.id)
            .get_result(conn)
            .map_err(|e| e.into())
    }
}

impl Handler<GetOrder> for DbExecutor {
    type Result = Result<Order, Error>;

    fn handle(&mut self, msg: GetOrder, _: &mut Self::Context) -> Self::Result {
        use crate::schema::orders::dsl::*;
        let conn: &PgConnection = &self.0.get().unwrap();
        orders
            .find(msg.order_id)
            .get_result(conn)
            .map_err(|e| e.into())
    }
}

impl Handler<GetOrders> for DbExecutor {
    type Result = Result<Vec<Order>, Error>;

    fn handle(&mut self, msg: GetOrders, _: &mut Self::Context) -> Self::Result {
        use crate::schema::orders::dsl::*;
        let conn: &PgConnection = &self.0.get().unwrap();
        orders
            .filter(merchant_id.eq(msg.merchant_id))
            .offset(msg.offset)
            .limit(msg.limit)
            .load::<Order>(conn)
            .map_err(|e| e.into())
    }
}

impl Handler<GetPendingOrders> for DbExecutor {
    type Result = Result<Vec<Order>, Error>;

    fn handle(&mut self, _: GetPendingOrders, _: &mut Self::Context) -> Self::Result {
        use crate::schema::orders::dsl::*;
        let conn: &PgConnection = &self.0.get().unwrap();

        let unpaid_orders = orders
            .filter(status.eq(OrderStatus::Pending))
            .load::<Order>(conn)
            .map_err(|e| Error::Db(s!(e)))?;

        Ok(unpaid_orders)
    }
}

impl Handler<GetConfirmedOrders> for DbExecutor {
    type Result = Result<Vec<Order>, Error>;

    fn handle(&mut self, _: GetConfirmedOrders, _: &mut Self::Context) -> Self::Result {
        use crate::schema::orders::dsl::*;
        let conn: &PgConnection = &self.0.get().unwrap();

        let confirmed_orders = orders
            .filter(status.eq(OrderStatus::Confirmed))
            .load::<Order>(conn)
            .map_err(|e| Error::Db(s!(e)))?;

        Ok(confirmed_orders)
    }
}

impl Handler<CreateOrder> for DbExecutor {
    type Result = Result<Order, Error>;

    fn handle(&mut self, msg: CreateOrder, _: &mut Self::Context) -> Self::Result {
        use crate::schema::merchants::dsl::*;
        use crate::schema::orders::dsl::*;
        use crate::schema::rates::dsl::*;

        let conn: &PgConnection = &self.0.get().unwrap();

        if !merchants
            .find(msg.merchant_id.clone())
            .get_result::<Merchant>(conn)
            .is_ok()
        {
            return Err(Error::InvalidEntity("merchant".to_owned()));
        }

        let exch_rate = match rates
            .find(&msg.amount.currency.to_string())
            .get_result::<Rate>(conn)
            .optional()?
        {
            None => return Err(Error::UnsupportedCurrency(msg.amount.currency.to_string())),
            Some(v) => v,
        };

        let grins = msg.amount.convert_to(Currency::GRIN, exch_rate.rate);

        let new_order = Order {
            id: uuid::Uuid::new_v4(),
            external_id: msg.external_id,
            merchant_id: msg.merchant_id,
            email: msg.email,
            amount: msg.amount,
            grin_amount: grins.amount,
            status: OrderStatus::Unpaid,
            confirmations: msg.confirmations,
            created_at: Local::now().naive_local(),
            updated_at: Local::now().naive_local(),
            report_attempts: 0,
            next_report_attempt: None,
            reported: false,
            wallet_tx_id: None,
            wallet_tx_slate_id: None,
            message: msg.message,
            slate_messages: None,
        };

        diesel::insert_into(orders)
            .values(&new_order)
            .get_result(conn)
            .map_err(|e| e.into())
    }
}

impl Handler<UpdateOrderStatus> for DbExecutor {
    type Result = Result<Order, Error>;

    fn handle(&mut self, msg: UpdateOrderStatus, _: &mut Self::Context) -> Self::Result {
        use crate::schema::orders::dsl::*;
        let conn: &PgConnection = &self.0.get().unwrap();

        diesel::update(orders.filter(id.eq(msg.id)))
            .set((status.eq(msg.status), updated_at.eq(Utc::now().naive_utc())))
            .get_result(conn)
            .map_err(|e| e.into())
    }
}

impl Handler<UpdateOrderWithTxLog> for DbExecutor {
    type Result = Result<(), Error>;

    fn handle(&mut self, msg: UpdateOrderWithTxLog, _: &mut Self::Context) -> Self::Result {
        use crate::schema::orders::dsl::*;
        let conn: &PgConnection = &self.0.get().unwrap();

        diesel::update(orders.filter(id.eq(msg.order_id)))
            .set((
                wallet_tx_id.eq(msg.wallet_tx_id),
                wallet_tx_slate_id.eq(msg.wallet_tx_slate_id),
                slate_messages.eq(msg.messages),
            ))
            .get_result(conn)
            .map_err(|e| e.into())
            .map(|_: Order| ())
    }
}

impl Handler<RegisterRate> for DbExecutor {
    type Result = Result<(), Error>;

    fn handle(&mut self, msg: RegisterRate, _: &mut Self::Context) -> Self::Result {
        use crate::schema::rates::dsl::*;
        let conn: &PgConnection = &self.0.get().unwrap();

        for (currency, new_rate) in msg.rates {
            let new_rate = Rate {
                id: currency.to_uppercase(),
                rate: new_rate,
                updated_at: Local::now().naive_local(),
            };

            diesel::insert_into(rates)
                .values(&new_rate)
                .on_conflict(id)
                .do_update()
                .set(&new_rate)
                .get_result::<Rate>(conn)
                .map_err(|e| Error::from(e))?;
        }
        Ok(())
    }
}

impl Handler<ConfirmOrder> for DbExecutor {
    type Result = Result<(), Error>;

    fn handle(&mut self, msg: ConfirmOrder, _: &mut Self::Context) -> Self::Result {
        use crate::schema::merchants;
        use crate::schema::orders;
        let conn: &PgConnection = &self.0.get().unwrap();

        conn.transaction(|| {
            diesel::update(orders::table.filter(orders::columns::id.eq(msg.order.id)))
                .set((
                    orders::columns::status.eq(OrderStatus::Confirmed),
                    orders::columns::updated_at.eq(Utc::now().naive_utc()),
                ))
                .get_result(conn)
                .map(|_: Order| ())?;
            diesel::update(
                merchants::table.filter(merchants::columns::id.eq(msg.order.merchant_id)),
            )
            .set(
                (merchants::columns::balance
                    .eq(merchants::columns::balance + msg.order.grin_amount)),
            )
            .get_result(conn)
            .map(|_: Merchant| ())?;
            Ok(())
        })
    }
}

impl Handler<ReportAttempt> for DbExecutor {
    type Result = Result<(), Error>;

    fn handle(&mut self, msg: ReportAttempt, _: &mut Self::Context) -> Self::Result {
        use crate::schema::orders::dsl::*;
        let conn: &PgConnection = &self.0.get().unwrap();
        let next_attempt = msg
            .next_attempt
            .unwrap_or(Utc::now().naive_utc() + Duration::seconds(10));
        diesel::update(orders.filter(id.eq(msg.order_id)))
            .set((
                report_attempts.eq(report_attempts + 1),
                next_report_attempt.eq(next_attempt),
            ))
            .get_result(conn)
            .map_err(|e| e.into())
            .map(|_: Order| ())
    }
}

impl Handler<MarkAsReported> for DbExecutor {
    type Result = Result<(), Error>;

    fn handle(&mut self, msg: MarkAsReported, _: &mut Self::Context) -> Self::Result {
        info!("Mark order {} as reported", msg.order_id);
        use crate::schema::orders::dsl::*;
        let conn: &PgConnection = &self.0.get().unwrap();
        diesel::update(orders.filter(id.eq(msg.order_id)))
            .set((reported.eq(true),))
            .get_result(conn)
            .map_err(|e| e.into())
            .map(|_: Order| ())
    }
}

impl Handler<GetUnreportedOrders> for DbExecutor {
    type Result = Result<Vec<Order>, Error>;

    fn handle(&mut self, _: GetUnreportedOrders, _: &mut Self::Context) -> Self::Result {
        use crate::schema::orders::dsl::*;
        let conn: &PgConnection = &self.0.get().unwrap();

        let query = orders
            .filter(reported.ne(true))
            .filter(status.eq_any(vec![OrderStatus::Confirmed, OrderStatus::Rejected]))
            .filter(report_attempts.lt(MAX_REPORT_ATTEMPTS))
            .filter(
                next_report_attempt
                    .le(Utc::now().naive_utc())
                    .or(next_report_attempt.is_null()),
            );

        let confirmed_orders = query.load::<Order>(conn).map_err(|e| Error::Db(s!(e)))?;

        Ok(confirmed_orders)
    }
}

impl Handler<Confirm2FA> for DbExecutor {
    type Result = Result<(), Error>;

    fn handle(&mut self, msg: Confirm2FA, _: &mut Self::Context) -> Self::Result {
        info!("Confirm 2fa token for merchant {}", msg.merchant_id);
        use crate::schema::merchants::dsl::*;
        let conn: &PgConnection = &self.0.get().unwrap();
        diesel::update(merchants.filter(id.eq(msg.merchant_id)))
            .set((confirmed_2fa.eq(true),))
            .get_result(conn)
            .map_err(|e| e.into())
            .map(|_: Merchant| ())
    }
}

impl Handler<Reset2FA> for DbExecutor {
    type Result = Result<(), Error>;

    fn handle(&mut self, msg: Reset2FA, _: &mut Self::Context) -> Self::Result {
        info!("Confirm 2fa token for merchant {}", msg.merchant_id);
        use crate::schema::merchants::dsl::*;
        let conn: &PgConnection = &self.0.get().unwrap();

        let new_token_2fa = BASE32.encode(&thread_rng().gen::<[u8; 10]>());
        diesel::update(merchants.filter(id.eq(msg.merchant_id)))
            .set((confirmed_2fa.eq(false), token_2fa.eq(new_token_2fa)))
            .get_result(conn)
            .map_err(|e| e.into())
            .map(|_: Merchant| ())
    }
}
