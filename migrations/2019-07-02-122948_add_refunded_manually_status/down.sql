-- This file should undo anything in `up.sql`
DELETE FROM pg_enum WHERE enumlabel = 'refunded_manually' AND enumtypid = (SELECT oid FROM pg_type WHERE typname = 'transaction_status');
