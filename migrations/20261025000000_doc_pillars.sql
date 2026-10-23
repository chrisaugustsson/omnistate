-- US-101: every doc sits in one of the five pillars, the same at the app level
-- and in every feature: spec, ui, architecture, db and research. The column
-- `module` becomes `pillar`; the old value `prd` becomes `spec` and the other
-- old values stay. No table is added, so the checklist of migrations/README.md
-- does not apply.
ALTER TABLE docs DROP CONSTRAINT docs_module_known;
ALTER TABLE docs RENAME COLUMN module TO pillar;
UPDATE docs SET pillar = 'spec' WHERE pillar = 'prd';
ALTER TABLE docs
    ADD CONSTRAINT docs_pillar_known CHECK (pillar IN ('spec', 'ui', 'architecture', 'db', 'research'));
