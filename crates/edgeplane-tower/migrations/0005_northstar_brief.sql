-- 0005_northstar_brief.sql — Northstar S3 path stub + Mission BRIEF columns
--
-- Domain: stub s3 path column for future S3 backing (content stays inline for now)
ALTER TABLE domain ADD COLUMN IF NOT EXISTS northstar_s3_path TEXT;

-- Mission: add brief columns (BRIEF replaces WORKSTREAM conceptually)
-- workstream_md stays for backward compatibility; brief_md is the new first-class column
ALTER TABLE mission ADD COLUMN IF NOT EXISTS brief_md TEXT NOT NULL DEFAULT '';
ALTER TABLE mission ADD COLUMN IF NOT EXISTS brief_version INTEGER NOT NULL DEFAULT 0;
ALTER TABLE mission ADD COLUMN IF NOT EXISTS brief_created_by TEXT NOT NULL DEFAULT '';
ALTER TABLE mission ADD COLUMN IF NOT EXISTS brief_modified_by TEXT NOT NULL DEFAULT '';
ALTER TABLE mission ADD COLUMN IF NOT EXISTS brief_created_at TIMESTAMP;
ALTER TABLE mission ADD COLUMN IF NOT EXISTS brief_modified_at TIMESTAMP;
ALTER TABLE mission ADD COLUMN IF NOT EXISTS brief_s3_path TEXT;
