-- This file should undo anything in `up.sql`
ALTER TABLE orders DROP COLUMN tx_id; 
ALTER TABLE orders DROP COLUMN tx_slate_id;
ALTER TABLE orders DROP COLUMN message;


