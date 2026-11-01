-- Wiki-link resolution matches titles with LOWER(title) per owner on every
-- note write; without this each of up to 50 links scans the owner's notes.
CREATE INDEX idx_notes_owner_lower_title ON notes (owner_id, LOWER(title));
