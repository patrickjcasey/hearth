-- Recipes a user explicitly chose to share as a link. Each row holds the
-- hestia_core::SharedRecipe JSON (no notes, ids or photos) and is deleted
-- once it expires. Nothing links a share to an install or a person.
CREATE TABLE shares (
    id          TEXT PRIMARY KEY,
    recipe      TEXT NOT NULL,
    created_at  TEXT NOT NULL,
    expires_at  TEXT NOT NULL
);

CREATE INDEX shares_created ON shares (created_at);
CREATE INDEX shares_expires ON shares (expires_at);
