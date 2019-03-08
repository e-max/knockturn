-- This file should undo anything in `up.sql`
ALTER TABLE TRANSACTIONS DROP COLUMN knockturn_fee;
ALTER TABLE TRANSACTIONS DROP COLUMN transfer_fee;
ALTER TABLE TRANSACTIONS DROP COLUMN real_transfer_fee;
