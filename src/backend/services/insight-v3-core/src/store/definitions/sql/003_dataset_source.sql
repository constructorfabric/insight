-- Every dataset declared before a dataset could be over anything but a stream.
--
-- The declaration now says what its dataset is over, and says it as a
-- required property: too much follows from it to leave to the presence of
-- another. A body written before that carries no `source` at all, and every
-- path that reads one would answer that the dataset cannot be read - while
-- the catalogue went on listing it, which is the worst shape a failure can
-- take.
--
-- Every such dataset is over a stream, because nothing else could be
-- declared then.

UPDATE datasets
SET body = JSON_SET(body, '$.source', JSON_OBJECT('kind', 'stream'))
WHERE JSON_EXTRACT(body, '$.source') IS NULL;
