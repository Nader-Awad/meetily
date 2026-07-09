-- Generalize NeoHive connection auth beyond the single Cloudflare Access method.
-- neohiveAuthType: cloudflare_access | bearer | basic | custom_header | none
-- neohiveAuthConfig: JSON object of method-specific fields (camelCase keys).
ALTER TABLE settings ADD COLUMN neohiveAuthType TEXT;
ALTER TABLE settings ADD COLUMN neohiveAuthConfig TEXT;

-- Backfill any existing Cloudflare Access config so it keeps working unchanged.
UPDATE settings
SET neohiveAuthType = 'cloudflare_access',
    neohiveAuthConfig = json_object('clientId', neohiveAccessClientId, 'clientSecret', neohiveAccessClientSecret)
WHERE neohiveAccessClientId IS NOT NULL OR neohiveAccessClientSecret IS NOT NULL;
