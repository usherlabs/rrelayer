BEGIN;

-- Stop relayer writers, then run with:
-- psql "$DATABASE_URL" -v ON_ERROR_STOP=1 -f scripts/migrate-legacy-block-hashes.sql
-- The lock prevents a legacy process from inserting another ASCII hash between
-- conversion and validation if a writer was not stopped cleanly.
LOCK TABLE relayer.transaction, relayer.transaction_audit_log
    IN SHARE ROW EXCLUSIVE MODE;

UPDATE relayer.transaction
SET block_hash = decode(substring(encode(block_hash, 'escape') FROM 3), 'hex')
WHERE octet_length(block_hash) = 66
  AND encode(block_hash, 'escape') ~ '^0x[0-9A-Fa-f]{64}$';

UPDATE relayer.transaction_audit_log
SET block_hash = decode(substring(encode(block_hash, 'escape') FROM 3), 'hex')
WHERE octet_length(block_hash) = 66
  AND encode(block_hash, 'escape') ~ '^0x[0-9A-Fa-f]{64}$';

DO $$
BEGIN
    IF EXISTS (
        SELECT 1
        FROM relayer.transaction
        WHERE block_hash IS NOT NULL
          AND octet_length(block_hash) <> 32
    ) OR EXISTS (
        SELECT 1
        FROM relayer.transaction_audit_log
        WHERE block_hash IS NOT NULL
          AND octet_length(block_hash) <> 32
    ) THEN
        RAISE EXCEPTION
            'block-hash migration refused: non-null values with invalid lengths remain';
    END IF;
END;
$$;

COMMIT;
