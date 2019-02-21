CREATE TABLE orders (
  order_id TEXT NOT NULL,
  merchant_id TEXT NOT NULL,
  grin_amount BIGINT NOT NULL,
  amount JSONB NOT NULL,
  status INTEGER NOT NULL,
  confirmations INTEGER NOT NULL,
  email TEXT,
  created_at TIMESTAMP NOT NULL,
  updated_at TIMESTAMP NOT NULL,
  PRIMARY KEY (merchant_id, order_id),
  FOREIGN KEY (merchant_id) REFERENCES merchants (id)
);
