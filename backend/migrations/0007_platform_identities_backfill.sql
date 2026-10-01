-- Guard: refuse to migrate links whose account no longer exists (names the rows).
DO $$
DECLARE orphans TEXT;
BEGIN
    SELECT string_agg(l.mxid, ', ') INTO orphans
    FROM matrix_links l LEFT JOIN accounts a ON a.id = l.account_id
    WHERE a.id IS NULL;
    IF orphans IS NOT NULL THEN
        RAISE EXCEPTION 'matrix_links rows reference missing accounts: %', orphans;
    END IF;
END $$;

INSERT INTO platform_identities (account_id, platform, platform_user_id, created_at, updated_at)
SELECT account_id, 'matrix', mxid, COALESCE(linked_at, now()), COALESCE(linked_at, now())
FROM matrix_links;

DROP TABLE matrix_links;
