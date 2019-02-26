-- Your SQL goes here

ALTER TABLE orders ADD COLUMN report_attempts int NOT NULL DEFAULT 0;
ALTER TABLE orders ADD COLUMN last_report_attempt TIMESTAMP;
