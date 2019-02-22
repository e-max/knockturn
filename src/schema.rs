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
    orders (id) {
        id -> Uuid,
        external_id -> Text,
        merchant_id -> Text,
        grin_amount -> Int8,
        amount -> Jsonb,
        status -> Int4,
        confirmations -> Int4,
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

table! {
    txs (slate_id) {
        slate_id -> Text,
        created_at -> Timestamp,
        confirmed -> Bool,
        confirmed_at -> Nullable<Timestamp>,
        fee -> Nullable<Int8>,
        messages -> Array<Text>,
        num_inputs -> Int8,
        num_outputs -> Int8,
        tx_type -> Text,
        order_id -> Uuid,
        updated_at -> Timestamp,
    }
}

joinable!(orders -> merchants (merchant_id));
joinable!(txs -> orders (order_id));

allow_tables_to_appear_in_same_query!(
    merchants,
    orders,
    rates,
    txs,
);
