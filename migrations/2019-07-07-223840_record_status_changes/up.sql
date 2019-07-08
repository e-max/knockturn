-- Your SQL goes here

CREATE EXTENSION IF NOT EXISTS "uuid-ossp";

CREATE TABLE status_changes (
  	id UUID PRIMARY KEY DEFAULT uuid_generate_v4(),
	transaction_id uuid NOT NULL, 
	status transaction_status NOT NULL, 
	updated_at TIMESTAMP NOT NULL,
  	FOREIGN KEY (transaction_id) REFERENCES transactions (id)
);

CREATE INDEX status_tx_idx ON status_changes(transaction_id);

CREATE OR REPLACE LANGUAGE plpgsql;

CREATE FUNCTION record_status_change() RETURNS trigger AS $record_status_change$
    BEGIN
        INSERT INTO status_changes (transaction_id, status, updated_at) 
        	VALUES (NEW.id, NEW.status, NEW.updated_at);
        RETURN NEW;
    END;
$record_status_change$ LANGUAGE plpgsql;

CREATE TRIGGER record_status_change AFTER INSERT OR UPDATE ON transactions
    FOR EACH ROW EXECUTE PROCEDURE record_status_change();
