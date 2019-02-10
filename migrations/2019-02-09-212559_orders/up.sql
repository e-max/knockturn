CREATE TABLE orders (
  order_id TEXT NOT NULL,
  merchant_id UUID NOT NULL,
  fiat_amount INTEGER NOT NULL,
  currency TEXT NOT NULL,
  amount INTEGER NOT NULL,
  status TEXT NOT NULL,
  created_at TIMESTAMP NOT NULL,
  PRIMARY KEY (merchant_id, order_id),
  FOREIGN KEY (merchant_id) REFERENCES merchants (id)
);
