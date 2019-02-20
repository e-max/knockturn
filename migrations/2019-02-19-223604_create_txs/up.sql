-- Your SQL goes here
CREATE TABLE txs (
  slate_id TEXT PRIMARY KEY,
  created_at TIMESTAMP NOT NULL,
  confirmed BOOLEAN NOT NULL DEFAULT 'f',
  confirmed_at TIMESTAMP,
  fee int8,
  messages TEXT[] NOT NULL,
  num_inputs int8 NOT NULL DEFAULT 0,
  num_outputs int8 NOT NULL DEFAULT 0,
  tx_type TEXT NOT NULL,
  merchant_id TEXT NOT NULL,
  order_id TEXT  NOT NULL,
  updated_at TIMESTAMP NOT NULL,
  FOREIGN KEY (merchant_id, order_id) REFERENCES orders (merchant_id, order_id)
)
