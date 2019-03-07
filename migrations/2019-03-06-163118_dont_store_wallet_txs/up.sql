-- Your SQL goes here
ALTER TABLE orders ADD COLUMN tx_id TEXT; 
ALTER TABLE orders ADD COLUMN tx_slate_id TEXT;
ALTER TABLE orders ADD COLUMN message TEXT NOT NULL DEFAULT '';

UPDATE orders SET tx_slate_id = (SELECT slate_id FROM txs WHERE order_id = orders.id);

