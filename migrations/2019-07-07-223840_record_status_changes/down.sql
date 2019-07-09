-- This file should undo anything in `up.sql`

DROP INDEX status_tx_idx;
DROP TRIGGER record_status_change ON transactions;
DROP FUNCTION record_status_change;
DROP TABLE status_changes;
