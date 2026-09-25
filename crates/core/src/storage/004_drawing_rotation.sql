-- View screen: the user's rotation of a drawing page, in degrees clockwise
-- (0, 90, 180 or 270), applied when displaying it. The PNG file is not
-- changed.
ALTER TABLE drawings ADD COLUMN rotation INTEGER NOT NULL DEFAULT 0;
