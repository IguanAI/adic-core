-- ADIC-DAG PostgreSQL Initialization Script
-- Sets up database schema for future state queries

-- Create extension for UUID support
CREATE EXTENSION IF NOT EXISTS "uuid-ossp";

-- Messages table (future implementation)
CREATE TABLE IF NOT EXISTS messages (
    id VARCHAR(64) PRIMARY KEY,
    timestamp BIGINT NOT NULL,
    sender VARCHAR(64) NOT NULL,
    payload TEXT,
    created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
    INDEX idx_timestamp (timestamp),
    INDEX idx_sender (sender)
);

-- Finality artifacts table (future implementation)
CREATE TABLE IF NOT EXISTS finality_artifacts (
    message_id VARCHAR(64) PRIMARY KEY,
    f1_finalized BOOLEAN DEFAULT FALSE,
    f1_timestamp BIGINT,
    f2_stabilized BOOLEAN DEFAULT FALSE,
    f2_timestamp BIGINT,
    k_core INTEGER,
    depth INTEGER,
    created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
    FOREIGN KEY (message_id) REFERENCES messages(id)
);

-- Account balances table (future implementation)
CREATE TABLE IF NOT EXISTS account_balances (
    address VARCHAR(64) PRIMARY KEY,
    balance BIGINT NOT NULL DEFAULT 0,
    reputation DOUBLE PRECISION DEFAULT 0,
    last_updated TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
    INDEX idx_balance (balance),
    INDEX idx_reputation (reputation)
);

-- Network statistics table
CREATE TABLE IF NOT EXISTS network_stats (
    id SERIAL PRIMARY KEY,
    timestamp TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
    total_messages BIGINT,
    f1_finalized_count BIGINT,
    f2_finalized_count BIGINT,
    active_validators INTEGER,
    avg_confirmation_time_ms INTEGER
);

-- Indices for query performance
CREATE INDEX IF NOT EXISTS idx_stats_timestamp ON network_stats(timestamp);

-- Insert initial placeholder data
INSERT INTO network_stats (total_messages, f1_finalized_count, f2_finalized_count, active_validators, avg_confirmation_time_ms)
VALUES (0, 0, 0, 0, 0);

-- Grant permissions
GRANT ALL PRIVILEGES ON ALL TABLES IN SCHEMA public TO adic;
GRANT ALL PRIVILEGES ON ALL SEQUENCES IN SCHEMA public TO adic;
