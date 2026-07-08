-- Cloudflare Access service-token credentials for the NeoHive MCP endpoint.
-- (Supersedes the earlier single neohiveApiKey column, which is left vestigial.)
ALTER TABLE settings ADD COLUMN neohiveAccessClientId TEXT;
ALTER TABLE settings ADD COLUMN neohiveAccessClientSecret TEXT;
