-- This file should undo anything in `up.sql`
ALTER TABLE merchants ADD COLUMN balance BIGINT NOT NULL DEFAULT 0;

