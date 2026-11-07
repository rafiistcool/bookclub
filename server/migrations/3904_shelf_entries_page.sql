-- Progress in pages (docs/phase3/notes/reading-trust.md): a reader who counts in pages saves the
-- page they are on, not a percent the client rounded. `progress` stays the measure everyone else
-- reads (rings, the club's readers, the spoiler shield, the posts' `progress_at`); with a page and
-- a known page count the server derives it from the page (shelf::entries::apply_page).

ALTER TABLE shelf_entries
    ADD COLUMN current_page integer NULL CHECK (current_page >= 0);
ALTER TABLE shelf_entries ADD CONSTRAINT shelf_entries_page_reading
    CHECK (current_page IS NULL OR status = 'currently_reading');

COMMENT ON COLUMN shelf_entries.current_page IS
    'The page the reader is on, while reading a book with a known page count (the entry''s edition''s, else the work''s); never above it. progress is derived from it on every write that sets it. NULL = the reader counts in percent (or the book has no page count): progress alone holds the position.';
