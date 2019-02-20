table! {
    merchants (id) {
        id -> Text,
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
        merchant_id -> Text,
        fiat_amount -> Int8,
        currency -> Text,
        amount -> Int8,
        status -> Int4,
        confirmations -> Int4,
        callback_url -> Text,
        email -> Nullable<Text>,
        created_at -> Timestamp,
        updated_at -> Timestamp,
    }
}

table! {
    rates (id) {
        id -> Text,
        rate -> Float8,
        updated_at -> Timestamp,
    }
}

joinable!(orders -> merchants (merchant_id));

allow_tables_to_appear_in_same_query!(
    merchants,
    orders,
    rates,
);
