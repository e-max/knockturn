table! {
    merchants (id) {
        id -> Uuid,
        email -> Varchar,
        password -> Varchar,
        wallet_url -> Nullable<Text>,
        balance -> Int8,
        created_at -> Timestamp,
    }
}

table! {
    orders (merchant_id, order_id) {
        order_id -> Text,
        merchant_id -> Uuid,
        fiat_amount -> Int4,
        currency -> Text,
        amount -> Int4,
        status -> Text,
        created_at -> Timestamp,
    }
}

joinable!(orders -> merchants (merchant_id));

allow_tables_to_appear_in_same_query!(
    merchants,
    orders,
);
