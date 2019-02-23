use crate::errors::*;
use crate::models::{Currency, Merchant, Money, Order, OrderStatus, Rate, Tx};
use actix::{Actor, SyncContext};
use actix::{Handler, Message};
use chrono::NaiveDateTime;
use chrono::{Duration, Local};
use diesel::pg::PgConnection;
use diesel::r2d2::{ConnectionManager, Pool};
use diesel::{self, prelude::*};
use serde::Deserialize;
use std::collections::HashMap;
use uuid::Uuid;

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
}

#[derive(Debug, Deserialize)]
pub struct GetMerchant {
    pub id: String,
}

#[derive(Debug, Deserialize)]
pub struct GetOrder {
    pub id: Uuid,
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

pub struct GetTxs {
    pub order_id: Uuid,
}

#[derive(Debug, Deserialize)]
pub struct GetTx {
    pub slate_id: String,
    pub order_id: Uuid,
}

#[derive(Debug, Deserialize)]
pub struct CreateTx {
    pub slate_id: String,
    pub created_at: NaiveDateTime,
    pub confirmed: bool,
    pub confirmed_at: Option<NaiveDateTime>,
    pub fee: Option<i64>,
    pub messages: Vec<String>,
    pub num_inputs: i64,
    pub num_outputs: i64,
    pub tx_type: String,
    pub order_id: Uuid,
}

#[derive(Debug, Deserialize)]
pub struct UpdateTx {
    pub slate_id: String,
    pub created_at: NaiveDateTime,
    pub confirmed: bool,
    pub confirmed_at: Option<NaiveDateTime>,
    pub fee: Option<i64>,
    pub messages: Vec<String>,
    pub num_inputs: i64,
    pub num_outputs: i64,
    pub order_id: Uuid,
}

impl From<Tx> for UpdateTx {
    fn from(tx: Tx) -> Self {
        UpdateTx {
            slate_id: tx.slate_id,
            created_at: tx.created_at,
            confirmed: tx.confirmed,
            confirmed_at: tx.confirmed_at,
            fee: tx.fee,
            messages: tx.messages,
            num_inputs: tx.num_inputs,
            num_outputs: tx.num_outputs,
            order_id: tx.order_id,
        }
    }
}

#[derive(Debug, Deserialize)]
pub struct GetReceivedOrders;

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
    type Result = Result<(), Error>;
}

impl Message for RegisterRate {
    type Result = Result<(), Error>;
}

impl Message for ConvertCurrency {
    type Result = Result<Money, Error>;
}
impl Message for CreateTx {
    type Result = Result<Tx, Error>;
}
impl Message for UpdateTx {
    type Result = Result<(), Error>;
}

impl Message for GetTxs {
    type Result = Result<Vec<Tx>, Error>;
}

impl Message for GetTx {
    type Result = Result<Option<Tx>, Error>;
}

impl Message for GetReceivedOrders {
    type Result = Result<Vec<(Order, Vec<Tx>)>, Error>;
}

impl Handler<CreateMerchant> for DbExecutor {
    type Result = Result<Merchant, Error>;

    fn handle(&mut self, msg: CreateMerchant, _: &mut Self::Context) -> Self::Result {
        use crate::schema::merchants::dsl::*;
        let conn: &PgConnection = &self.0.get().unwrap();

        let new_merchant = Merchant {
            id: msg.id,
            email: msg.email,
            password: msg.password,
            wallet_url: msg.wallet_url,
            balance: 0,
            created_at: Local::now().naive_local() + Duration::hours(24),
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
        orders.find(msg.id).get_result(conn).map_err(|e| e.into())
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

impl Handler<GetReceivedOrders> for DbExecutor {
    type Result = Result<Vec<(Order, Vec<Tx>)>, Error>;

    fn handle(&mut self, msg: GetReceivedOrders, _: &mut Self::Context) -> Self::Result {
        use crate::schema::orders::dsl::*;
        let conn: &PgConnection = &self.0.get().unwrap();

        let unpaid_orders = orders
            .filter(status.eq(OrderStatus::Received))
            .load::<Order>(conn)
            .map_err(|e| Error::Db(s!(e)))?;

        let txs = Tx::belonging_to(&unpaid_orders)
            .load::<Tx>(conn)?
            .grouped_by(&unpaid_orders);
        let data = unpaid_orders.into_iter().zip(txs).collect::<Vec<_>>();
        Ok(data)
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
        };

        diesel::insert_into(orders)
            .values(&new_order)
            .get_result(conn)
            .map_err(|e| e.into())
    }
}

impl Handler<UpdateOrderStatus> for DbExecutor {
    type Result = Result<(), Error>;

    fn handle(&mut self, msg: UpdateOrderStatus, _: &mut Self::Context) -> Self::Result {
        use crate::schema::orders::dsl::*;
        let conn: &PgConnection = &self.0.get().unwrap();

        diesel::update(orders.filter(id.eq(msg.id)))
            .set((status.eq(msg.status),))
            .get_result(conn)
            .map_err(|e| e.into())
            .map(|order: Order| ())
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

impl Handler<CreateTx> for DbExecutor {
    type Result = Result<Tx, Error>;

    fn handle(&mut self, msg: CreateTx, _: &mut Self::Context) -> Self::Result {
        use crate::schema::txs::dsl::*;
        let conn: &PgConnection = &self.0.get().unwrap();

        let new_tx = Tx {
            slate_id: msg.slate_id,
            created_at: msg.created_at,
            confirmed: msg.confirmed,
            confirmed_at: msg.confirmed_at,
            fee: msg.fee,
            messages: msg.messages,
            num_inputs: msg.num_outputs,
            num_outputs: msg.num_outputs,
            tx_type: msg.tx_type,
            order_id: msg.order_id,
            updated_at: Local::now().naive_local(),
        };

        diesel::insert_into(txs)
            .values(&new_tx)
            .get_result(conn)
            .map_err(|e| e.into())
    }
}

impl Handler<UpdateTx> for DbExecutor {
    type Result = Result<(), Error>;

    fn handle(&mut self, msg: UpdateTx, _: &mut Self::Context) -> Self::Result {
        use crate::schema::txs::dsl::*;
        let conn: &PgConnection = &self.0.get().unwrap();

        diesel::update(txs.filter(slate_id.eq(msg.slate_id)))
            .set((
                created_at.eq(msg.created_at),
                confirmed.eq(msg.confirmed),
                confirmed_at.eq(msg.confirmed_at),
                fee.eq(msg.fee),
                messages.eq(msg.messages),
                num_inputs.eq(msg.num_outputs),
                num_outputs.eq(msg.num_outputs),
                updated_at.eq(Local::now().naive_local()),
            ))
            .get_result(conn)
            .map_err(|e| e.into())
            .map(|tx: Tx| ())
    }
}

impl Handler<GetTxs> for DbExecutor {
    type Result = Result<Vec<Tx>, Error>;

    fn handle(&mut self, msg: GetTxs, _: &mut Self::Context) -> Self::Result {
        use crate::schema::txs::dsl::*;
        let conn: &PgConnection = &self.0.get().unwrap();
        txs.filter(order_id.eq(msg.order_id))
            .load::<Tx>(conn)
            .map_err(|e| e.into())
    }
}

impl Handler<GetTx> for DbExecutor {
    type Result = Result<Option<Tx>, Error>;

    fn handle(&mut self, msg: GetTx, _: &mut Self::Context) -> Self::Result {
        use crate::schema::txs::dsl::*;
        let conn: &PgConnection = &self.0.get().unwrap();
        let sid = msg.slate_id.clone();
        txs.filter(order_id.eq(msg.order_id))
            .filter(slate_id.eq(msg.slate_id))
            .load::<Tx>(conn)
            .map_err(|e| e.into())
            .and_then(|transactions| match transactions.len() {
                0 => Ok(None),
                1 => Ok(transactions.into_iter().next()),
                _ => Err(Error::Db(format!(
                    "Found more than one transaction with slate id {}",
                    sid
                ))),
            })
    }
}
