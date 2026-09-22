-- Superseded by `pages` and `site.values_json` in 0006; the release that
-- read them is gone.
DROP TABLE project_assets;
DROP TABLE projects;

ALTER TABLE profile DROP COLUMN about_source;
ALTER TABLE profile DROP COLUMN about_html;
ALTER TABLE profile DROP COLUMN about_text;

ALTER TABLE site DROP COLUMN banner;
ALTER TABLE site DROP COLUMN descriptions;
ALTER TABLE site DROP COLUMN blog_enabled;
