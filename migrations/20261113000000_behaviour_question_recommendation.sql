-- The answer the asker recommends for a question it leaves open for a person.
-- It is set when the question is created and never changed afterwards, not
-- even when the question is answered; null when the asker recommended nothing.

ALTER TABLE behaviour_questions ADD COLUMN recommendation text
    CONSTRAINT behaviour_questions_recommendation_not_blank CHECK (btrim(recommendation) <> '');
